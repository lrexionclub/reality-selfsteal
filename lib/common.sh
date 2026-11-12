#!/usr/bin/env bash
# =============================================================================
# lib/common.sh —— 公共函数层
#   发行版探测 / 包管理 / 服务管理 / 防火墙 / Nginx 路径 / 模板渲染
#   被 install.sh 与 uninstall.sh 共同 source
# =============================================================================

C_RESET=$'\033[0m'
C_RED=$'\033[31m'
C_GREEN=$'\033[32m'
C_YELLOW=$'\033[33m'
C_BLUE=$'\033[36m'

log()  { printf '%s[xdd]%s %s\n'  "$C_BLUE"   "$C_RESET" "$*"; }
ok()   { printf '%s[ ok ]%s %s\n' "$C_GREEN"  "$C_RESET" "$*"; }
warn() { printf '%s[warn]%s %s\n' "$C_YELLOW" "$C_RESET" "$*" >&2; }
die()  { printf '%s[fail]%s %s\n' "$C_RED"    "$C_RESET" "$*" >&2; exit 1; }

have() { command -v "$1" >/dev/null 2>&1; }

# ---------------------------------------------------------------- 发行版探测
detect_os() {
  OS_ID=unknown
  OS_VER=""
  if [ -r /etc/os-release ]; then
    # shellcheck disable=SC1091
    . /etc/os-release
    OS_ID=${ID:-unknown}
    OS_VER=${VERSION_ID:-}
  fi

  case "$OS_ID" in
    alpine)                                     PKG=apk ;;
    debian|ubuntu|raspbian|armbian|linuxmint)   PKG=apt ;;
    centos|rhel|rocky|almalinux|fedora|ol)      PKG=dnf ;;
    *)
      if   have apk;     then PKG=apk
      elif have apt-get; then PKG=apt
      elif have dnf;     then PKG=dnf
      else die "未识别的发行版：${OS_ID}，请提 issue 附 /etc/os-release"
      fi ;;
  esac

  INIT=unknown
  if [ -f /sbin/openrc-run ] || [ -x /sbin/rc-service ]; then
    INIT=openrc
  fi
  if [ -d /run/systemd/system ] && have systemctl; then
    INIT=systemd
  fi
  [ "$INIT" = unknown ] && die "未识别的 init 系统（既无 systemd 也无 OpenRC）"

  case "$PKG" in
    apk) NGINX_USER=nginx;    NGINX_SITE_DIR=/etc/nginx/http.d ;;
    *)   NGINX_USER=www-data; NGINX_SITE_DIR=/etc/nginx/sites-available ;;
  esac

  export NGINX_USER NGINX_SITE_DIR
  ok "发行版 ${OS_ID} ${OS_VER:-?} / 包管理 ${PKG} / init ${INIT}"
}

# ---------------------------------------------------------------- 包管理
pkg_install() {
  [ "$#" -eq 0 ] && return 0
  case "$PKG" in
    apk) apk add --no-cache "$@" ;;
    apt) DEBIAN_FRONTEND=noninteractive apt-get update -qq
         DEBIAN_FRONTEND=noninteractive apt-get install -y -qq "$@" ;;
    dnf) dnf install -y -q "$@" ;;
  esac
}

pkg_remove() {
  [ "$#" -eq 0 ] && return 0
  case "$PKG" in
    apk) apk del "$@" ;;
    apt) DEBIAN_FRONTEND=noninteractive apt-get purge -y -qq "$@" ;;
    dnf) dnf remove -y -q "$@" ;;
  esac
}

# ---------------------------------------------------------------- 服务管理
svc_enable_start() {
  local name=$1
  case "$INIT" in
    systemd) systemctl enable --now "$name" ;;
    openrc)  rc-update add "$name" default >/dev/null 2>&1 || true
             rc-service "$name" restart ;;
  esac
}

svc_restart() {
  case "$INIT" in
    systemd) systemctl restart "$1" ;;
    openrc)  rc-service "$1" restart ;;
  esac
}

svc_reload() {
  local name=$1
  case "$INIT" in
    systemd) systemctl reload "$name" 2>/dev/null || systemctl restart "$name" ;;
    openrc)  rc-service "$name" reload 2>/dev/null || rc-service "$name" restart ;;
  esac
}

svc_stop_disable() {
  local name=$1
  case "$INIT" in
    systemd) systemctl disable --now "$name" 2>/dev/null || true ;;
    openrc)  rc-service "$name" stop 2>/dev/null || true
             rc-update del "$name" default 2>/dev/null || true ;;
  esac
}

svc_state() {
  case "$INIT" in
    systemd) systemctl is-active "$1" 2>/dev/null || true ;;
    openrc)  rc-service "$1" status 2>/dev/null | head -1 || true ;;
  esac
}

# ---------------------------------------------------------------- 防火墙
fw_apply() {
  local ssh_port=$1
  case "$PKG" in
    apk)
      pkg_install nftables
      cat > /etc/nftables.nft <<NFT
#!/usr/sbin/nft -f
flush ruleset
table inet filter {
    chain input {
        type filter hook input priority 0; policy drop;
        iif lo accept
        ct state established,related accept
        ct state invalid drop
        ip protocol icmp accept
        ip6 nexthdr ipv6-icmp accept
        tcp dport { ${ssh_port}, 80, 443 } accept
    }
    chain forward { type filter hook forward priority 0; policy drop; }
    chain output  { type filter hook output  priority 0; policy accept; }
}
NFT
      nft -c -f /etc/nftables.nft || die "nftables 规则校验失败，未做任何改动"
      rc-update add nftables default >/dev/null 2>&1 || true
      rc-service nftables restart || die "nftables 规则加载失败"
      ok "nftables 已生效：drop 入站，放行 ${ssh_port}/80/443"
      ;;
    apt)
      pkg_install ufw
      ufw --force reset >/dev/null 2>&1 || true
      ufw default deny incoming  >/dev/null
      ufw default allow outgoing >/dev/null
      ufw allow "${ssh_port}/tcp" >/dev/null
      ufw allow 80/tcp   >/dev/null
      ufw allow 443/tcp  >/dev/null
      ufw --force enable >/dev/null
      ok "ufw 已生效：放行 ${ssh_port}/80/443"
      ;;
    dnf)
      pkg_install firewalld
      systemctl enable --now firewalld >/dev/null 2>&1 || true
      firewall-cmd --permanent --add-port="${ssh_port}/tcp" >/dev/null
      firewall-cmd --permanent --add-service=http  >/dev/null
      firewall-cmd --permanent --add-service=https >/dev/null
      firewall-cmd --reload >/dev/null
      ok "firewalld 已生效：放行 ${ssh_port}/80/443"
      ;;
  esac
}

fw_remove() {
  case "$PKG" in
    apk) rc-service nftables stop 2>/dev/null || true
         rc-update del nftables default 2>/dev/null || true
         rm -f /etc/nftables.nft ;;
    apt) ufw --force disable 2>/dev/null || true
         ufw --force reset   2>/dev/null || true ;;
    dnf) firewall-cmd --permanent --remove-port="${1:-22}/tcp" >/dev/null 2>&1 || true
         firewall-cmd --reload >/dev/null 2>&1 || true ;;
  esac
}

# ---------------------------------------------------------------- 模板渲染
# 用法：tpl <模板文件> <目标文件> ；变量来自全局
tpl() {
  local src=$1 dst=$2
  sed \
    -e "s|@DOMAIN@|${DOMAIN:-}|g" \
    -e "s|@TLS_PORT@|${TLS_PORT:-8443}|g" \
    -e "s|@SITE_ROOT@|${SITE_ROOT:-}|g" \
    -e "s|@ACME_WEBROOT@|${ACME_WEBROOT:-}|g" \
    -e "s|@SSL_DIR@|${SSL_DIR:-}|g" \
    -e "s|@NGINX_USER@|${NGINX_USER:-www-data}|g" \
    -e "s|@LISTEN_HTTP2@|${LISTEN_HTTP2:-}|g" \
    -e "s|@HTTP2_ON@|${HTTP2_ON:-}|g" \
    -e "s|@UUID@|${UUID:-}|g" \
    -e "s|@PRIVKEY@|${PRIV:-}|g" \
    -e "s|@SHORTID@|${SHORTID:-}|g" \
    -e "s|@XPATH@|${XPATH:-}|g" \
    "$src" > "$dst"
}

# ---------------------------------------------------------------- 杂项
rand_hex() {
  local n=${1:-8}
  if have openssl; then
    openssl rand -hex "$n"
  else
    od -An -tx1 -N"$n" /dev/urandom | tr -d ' \n'
  fi
}

public_ip() {
  local ip=""
  for url in https://api.ipify.org https://ifconfig.me/ip https://ipv4.icanhazip.com; do
    ip=$(curl -4 -fsS --max-time 6 "$url" 2>/dev/null | tr -d '[:space:]') || true
    case "$ip" in
      *[0-9].[0-9]*) printf '%s' "$ip"; return 0 ;;
    esac
  done
  printf '%s' "[你的服务器IP]"
}

nginx_version_num() {
  local v
  v=$(nginx -v 2>&1 | sed -n 's|.*nginx/||p' | cut -d- -f1)
  [ -z "$v" ] && { printf '0'; return; }
  printf '%s' "$v"
}

ver_ge() { # ver_ge <a> <b>  → a >= b ?（纯 bash，不依赖 sort -V，busybox 也能跑）
  local a=$1 b=$2 i x y
  local -a av bv
  IFS=. read -r -a av <<< "$a"
  IFS=. read -r -a bv <<< "$b"
  for i in 0 1 2; do
    x=${av[i]:-0}; x=${x//[^0-9]/}
    y=${bv[i]:-0}; y=${y//[^0-9]/}
    x=${x:-0}; y=${y:-0}
    if [ "$((10#$x))" -gt "$((10#$y))" ]; then return 0; fi
    if [ "$((10#$x))" -lt "$((10#$y))" ]; then return 1; fi
  done
  return 0
}
