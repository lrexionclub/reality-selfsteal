#!/usr/bin/env bash
# =============================================================================
#  rs —— REALITY「偷自己」单文件安装器（steal oneself）
#
#  把 Nginx 退回 127.0.0.1:8443，Xray 独占 0.0.0.0:443，
#  dest 指向本机真站点：域名、证书、网站全在同一台机器上，探测怎么查都对得上。
#
#  支持：Alpine(apk/OpenRC) · Debian/Ubuntu(apt/systemd) · RHEL 系(dnf/systemd)
#  架构：x86_64 / aarch64 / armv7l / i386 / s390x
#
#  单行安装：
#  (curl -LfsS https://raw.githubusercontent.com/lrexionclub/reality-selfsteal/main/rs \
#     -o /usr/local/bin/rs || wget -q https://raw.githubusercontent.com/lrexionclub/reality-selfsteal/main/rs \
#     -O /usr/local/bin/rs) && chmod +x /usr/local/bin/rs && rs
#
#  子命令：
#    rs                    进入交互菜单
#    rs install --domain a.example.com --email you@mail.com [--network xhttp]
#    rs info               打印节点信息与客户端链接
#    rs check              自检（端口 / 回落 / 证书链）
#    rs upgrade            升级 Xray 内核
#    rs uninstall [--all]  卸载
#    rs self-update        重新下载本脚本
# =============================================================================

# --- Alpine/busybox 默认没有 bash，先自举再重入 -------------------------------
if [ -z "${BASH_VERSION:-}" ]; then
  if command -v apk >/dev/null 2>&1; then
    apk add --no-cache bash >/dev/null 2>&1 || true
  fi
  if command -v bash >/dev/null 2>&1; then
    exec bash "$0" "$@"
  fi
  echo "[fail] 本脚本需要 bash，请先 apk add bash" >&2
  exit 1
fi

set -uo pipefail

# ---------------------------------------------------------------- 常量
RS_VERSION="1.0.0"
REPO_SLUG="${RS_REPO_SLUG:-lrexionclub/reality-selfsteal}"
RAW_BASE="https://raw.githubusercontent.com/${REPO_SLUG}/main"
FALLBACK_XRAY="v26.11.9"

CONF_DIR=/usr/local/etc/xray
CONF_FILE=${CONF_DIR}/config.json
BIN=/usr/local/bin/xray
LOG_DIR=/var/log/xray
INFO_FILE=/root/reality-selfsteal.info
ACME_HOME=/root/.acme.sh
ACME_WEBROOT=/var/www/acme

DOMAIN=""
EMAIL=""
NETWORK="raw"
TLS_PORT=8443
SSH_PORT=22
CF_TOKEN=""
ACME_MODE="webroot"
PKG=""
INIT=""
NGINX_USER=""
NGINX_SITE_DIR=""
DO_CERT=1
DO_FIREWALL=1
ASSUME_YES=0
XRAY_VERSION=""

# ---------------------------------------------------------------- 输出
if [ -t 1 ]; then
  C_R=$'\033[31m'; C_G=$'\033[32m'; C_Y=$'\033[33m'; C_B=$'\033[36m'; C_D=$'\033[2m'; C_0=$'\033[0m'
else
  C_R=""; C_G=""; C_Y=""; C_B=""; C_D=""; C_0=""
fi
log()  { printf '%s[rs]%s %s\n'  "$C_B" "$C_0" "$*"; }
ok()   { printf '%s[ok]%s %s\n'  "$C_G" "$C_0" "$*"; }
warn() { printf '%s[!!]%s %s\n'  "$C_Y" "$C_0" "$*" >&2; }
die()  { printf '%s[xx]%s %s\n'  "$C_R" "$C_0" "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

need_root() { [ "$(id -u)" -eq 0 ] || die "需要 root 权限"; }

# ---------------------------------------------------------------- 发行版
detect_os() {
  OS_ID=unknown; OS_VER=""
  if [ -r /etc/os-release ]; then
    # shellcheck disable=SC1091
    . /etc/os-release
    OS_ID=${ID:-unknown}; OS_VER=${VERSION_ID:-}
  fi
  case "$OS_ID" in
    alpine)                                   PKG=apk ;;
    debian|ubuntu|raspbian|armbian|linuxmint) PKG=apt ;;
    centos|rhel|rocky|almalinux|fedora|ol)    PKG=dnf ;;
    *)
      if   have apk;     then PKG=apk
      elif have apt-get; then PKG=apt
      elif have dnf;     then PKG=dnf
      else die "未识别的发行版：${OS_ID}"
      fi ;;
  esac

  INIT=unknown
  if have systemctl && [ -d /run/systemd/system ]; then INIT=systemd; fi
  if [ -x /sbin/rc-service ] || [ -f /sbin/openrc-run ]; then INIT=openrc; fi
  [ "$INIT" = unknown ] && die "未识别的 init 系统"

  case "$PKG" in
    apk) NGINX_USER=nginx;    NGINX_SITE_DIR=/etc/nginx/http.d ;;
    *)   NGINX_USER=www-data; NGINX_SITE_DIR=/etc/nginx/sites-available ;;
  esac
}

pkg_install() {
  [ "$#" -eq 0 ] && return 0
  case "$PKG" in
    apk) apk add --no-cache "$@" ;;
    apt) DEBIAN_FRONTEND=noninteractive apt-get update -qq
         DEBIAN_FRONTEND=noninteractive apt-get install -y -qq "$@" ;;
    dnf) dnf install -y -q "$@" ;;
  esac
}

# ---------------------------------------------------------------- 服务
svc_start() {
  case "$INIT" in
    systemd) systemctl enable --now "$1" ;;
    openrc)  rc-update add "$1" default >/dev/null 2>&1 || true
             rc-service "$1" restart ;;
  esac
}
svc_restart() { case "$INIT" in systemd) systemctl restart "$1" ;; openrc) rc-service "$1" restart ;; esac; }
svc_reload()  { case "$INIT" in
                  systemd) systemctl reload "$1" 2>/dev/null || systemctl restart "$1" ;;
                  openrc)  rc-service "$1" reload 2>/dev/null || rc-service "$1" restart ;;
                esac; }
svc_stop()    { case "$INIT" in
                  systemd) systemctl disable --now "$1" 2>/dev/null || true ;;
                  openrc)  rc-service "$1" stop 2>/dev/null || true
                           rc-update del "$1" default 2>/dev/null || true ;;
                esac; }
svc_state()   { case "$INIT" in
                  systemd) systemctl is-active "$1" 2>/dev/null ;;
                  openrc)  rc-service "$1" status 2>/dev/null | head -1 ;;
                esac; }

# ---------------------------------------------------------------- 防火墙
fw_apply() {
  case "$PKG" in
    apk)
      pkg_install nftables || die "nftables 安装失败"
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
        tcp dport { ${SSH_PORT}, 80, 443 } accept
    }
    chain forward { type filter hook forward priority 0; policy drop; }
    chain output  { type filter hook output  priority 0; policy accept; }
}
NFT
      nft -c -f /etc/nftables.nft || die "nftables 规则校验失败，未应用"
      rc-update add nftables default >/dev/null 2>&1 || true
      rc-service nftables restart || die "nftables 加载失败"
      ok "nftables 生效：drop 入站，放行 ${SSH_PORT}/80/443" ;;
    apt)
      pkg_install ufw || die "ufw 安装失败"
      ufw --force reset >/dev/null 2>&1 || true
      ufw default deny incoming >/dev/null
      ufw default allow outgoing >/dev/null
      ufw allow "${SSH_PORT}/tcp" >/dev/null
      ufw allow 80/tcp >/dev/null
      ufw allow 443/tcp >/dev/null
      ufw --force enable >/dev/null
      ok "ufw 生效：放行 ${SSH_PORT}/80/443" ;;
    dnf)
      pkg_install firewalld || die "firewalld 安装失败"
      systemctl enable --now firewalld >/dev/null 2>&1 || true
      firewall-cmd --permanent --add-port="${SSH_PORT}/tcp" >/dev/null
      firewall-cmd --permanent --add-service=http  >/dev/null
      firewall-cmd --permanent --add-service=https >/dev/null
      firewall-cmd --reload >/dev/null
      ok "firewalld 生效：放行 ${SSH_PORT}/80/443" ;;
  esac
}

fw_remove() {
  case "$PKG" in
    apk) rc-service nftables stop 2>/dev/null || true
         rc-update del nftables default 2>/dev/null || true
         rm -f /etc/nftables.nft ;;
    apt) ufw --force disable 2>/dev/null || true; ufw --force reset 2>/dev/null || true ;;
    dnf) firewall-cmd --reload >/dev/null 2>&1 || true ;;
  esac
}

# ---------------------------------------------------------------- 内嵌模板
tpl_index() {
  cat <<'HTML'
<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>API Service</title>
<style>
 :root{color-scheme:dark}
 body{margin:0;min-height:100vh;display:grid;place-items:center;background:#0f1115;color:#e6e6e6;
      font-family:system-ui,-apple-system,"Segoe UI",Roboto,sans-serif}
 .card{max-width:560px;padding:34px 36px;border:1px solid #23262d;border-radius:14px;background:#15181e}
 h1{font-size:19px;margin:0 0 14px;font-weight:600}
 p{line-height:1.75;color:#9aa4b2;font-size:14px;margin:0 0 10px}
 code{background:#0b0d11;padding:2px 6px;border-radius:5px;color:#7fd1ff;font-size:13px}
 .dot{display:inline-block;width:8px;height:8px;border-radius:50%;background:#39d353;margin-right:8px}
</style></head>
<body><div class="card">
<h1><span class="dot"></span>Status: OK</h1>
<p>Endpoint is up. Version <code>v3</code>. Served over TLS 1.3 with HTTP/2.</p>
<p>If you reached this page by accident, there is nothing interesting here.</p>
</div></body></html>
HTML
}

tpl_nginx() {
  local listen_http2=$1 http2_on=$2
  cat <<NGINX
server {
    listen 80;
    listen [::]:80;
    server_name ${DOMAIN};

    location /.well-known/acme-challenge/ { root ${ACME_WEBROOT}; }
    location / { return 301 https://\$host\$request_uri; }
}

server {
    # 127.0.0.1 前缀：只给本机 Xray 回落用，外部扫这个端口扫不到东西
    listen 127.0.0.1:${TLS_PORT} ssl${listen_http2};
${http2_on}
    server_name ${DOMAIN};

    ssl_certificate     /etc/nginx/ssl/${DOMAIN}/fullchain.pem;
    ssl_certificate_key /etc/nginx/ssl/${DOMAIN}/privkey.pem;
    ssl_protocols TLSv1.2 TLSv1.3;
    ssl_ciphers TLS_AES_128_GCM_SHA256:TLS_AES_256_GCM_SHA384:TLS_CHACHA20_POLY1305_SHA256:ECDHE-ECDSA-AES128-GCM-SHA256:ECDHE-RSA-AES128-GCM-SHA256;
    ssl_prefer_server_ciphers off;
    ssl_session_cache shared:SSL:10m;
    ssl_session_timeout 1d;
    ssl_session_tickets off;
    add_header Strict-Transport-Security "max-age=31536000" always;
    server_tokens off;

    root ${SITE_ROOT};
    index index.html;
    location / { try_files \$uri \$uri/ =404; }
    location /.well-known/acme-challenge/ { root ${ACME_WEBROOT}; }
}
NGINX
}

tpl_xray() {
  local flow_field="" xhttp_field=""
  if [ "$NETWORK" = xhttp ]; then
    xhttp_field=$(cat <<XH
        "xhttpSettings": {
          "path": "${XPATH}",
          "mode": "auto",
          "extra": { "xPaddingBytes": "100-1000", "scMaxEachPostBytes": 1000000 }
        },
XH
)
  else
    flow_field=',
            "flow": "xtls-rprx-vision"'
  fi
  cat <<JSON
{
  "log": {
    "loglevel": "warning",
    "access": "${LOG_DIR}/access.log",
    "error": "${LOG_DIR}/error.log"
  },
  "inbounds": [
    {
      "tag": "vless-reality-443",
      "listen": "0.0.0.0",
      "port": 443,
      "protocol": "vless",
      "settings": {
        "clients": [
          {
            "id": "${UUID}",
            "email": "user1@${DOMAIN}"${flow_field}
          }
        ],
        "decryption": "none"
      },
      "streamSettings": {
        "network": "$([ "$NETWORK" = xhttp ] && echo xhttp || echo raw)",
        "security": "reality",
        "realitySettings": {
          "show": false,
          "dest": "127.0.0.1:${TLS_PORT}",
          "xver": 0,
          "serverNames": [ "${DOMAIN}" ],
          "privateKey": "${PRIV}",
          "shortIds": [ "${SHORTID}", "" ]
        },
${xhttp_field}        "sockopt": { "tcpFastOpen": true, "tcpNoDelay": true, "tcpcongestion": "bbr" }
      },
      "sniffing": {
        "enabled": true,
        "destOverride": [ "http", "tls", "quic" ],
        "routeOnly": true
      }
    }
  ],
  "outbounds": [
    { "tag": "direct", "protocol": "freedom", "settings": { "domainStrategy": "UseIP" } },
    { "tag": "block",  "protocol": "blackhole" }
  ],
  "routing": {
    "domainStrategy": "AsIs",
    "rules": [
      { "type": "field", "ip": [ "geoip:private" ], "outboundTag": "block" },
      { "type": "field", "protocol": [ "bittorrent" ], "outboundTag": "block" }
    ]
  }
}
JSON
}

tpl_unit_systemd() {
  local grp=$1
  cat <<UNIT
[Unit]
Description=Xray Service (reality-selfsteal)
After=network.target nss-lookup.target nginx.service

[Service]
User=nobody
Group=${grp}
CapabilityBoundingSet=CAP_NET_ADMIN CAP_NET_BIND_SERVICE
AmbientCapabilities=CAP_NET_ADMIN CAP_NET_BIND_SERVICE
NoNewPrivileges=true
ExecStart=${BIN} run -config ${CONF_FILE}
Restart=on-failure
RestartPreventExitStatus=23
LimitNOFILE=1048576
RuntimeDirectory=xray
RuntimeDirectoryMode=0755

[Install]
WantedBy=multi-user.target
UNIT
}

tpl_init_openrc() {
  cat <<INIT
#!/sbin/openrc-run
name="xray"
description="Xray Service (reality-selfsteal)"
command="${BIN}"
command_args="run -config ${CONF_FILE}"
command_background=true
pidfile="/run/\${RC_SVCNAME}.pid"
output_log="${LOG_DIR}/xray.out.log"
error_log="${LOG_DIR}/xray.err.log"

depend() {
    need net
    after nginx
    use dns
}

start_pre() {
    checkpath --directory --mode 0755 ${LOG_DIR}
}
INIT
}

# ---------------------------------------------------------------- 工具
rand_hex() { openssl rand -hex "${1:-8}" 2>/dev/null || od -An -tx1 -N"${1:-8}" /dev/urandom | tr -d ' \n'; }

public_ip() {
  local ip=""
  for u in https://api.ipify.org https://ifconfig.me/ip https://ipv4.icanhazip.com; do
    ip=$(curl -4 -fsS --max-time 6 "$u" 2>/dev/null | tr -d '[:space:]') || true
    case "$ip" in *[0-9].[0-9]*) printf '%s' "$ip"; return 0 ;; esac
  done
  printf '[你的服务器IP]'
}

ver_ge() {
  local a=$1 b=$2 i x y; local -a av bv
  IFS=. read -r -a av <<< "$a"
  IFS=. read -r -a bv <<< "$b"
  for i in 0 1 2; do
    x=${av[i]:-0}; x=${x//[^0-9]/}; x=${x:-0}
    y=${bv[i]:-0}; y=${y//[^0-9]/}; y=${y:-0}
    [ "$((10#$x))" -gt "$((10#$y))" ] && return 0
    [ "$((10#$x))" -lt "$((10#$y))" ] && return 1
  done
  return 0
}

xray_asset() {
  case "$(uname -m)" in
    x86_64|amd64)  echo Xray-linux-64.zip ;;
    aarch64|arm64) echo Xray-linux-arm64-v8a.zip ;;
    armv7l|armv7)  echo Xray-linux-arm32-v7a.zip ;;
    i386|i686)     echo Xray-linux-32.zip ;;
    s390x)         echo Xray-linux-s390x.zip ;;
    *) die "未适配的架构：$(uname -m)" ;;
  esac
}

latest_xray_tag() {
  local t
  t=$(curl -fsS --max-time 10 https://api.github.com/repos/XTLS/Xray-core/releases/latest 2>/dev/null \
      | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -1) || true
  [ -n "${t:-}" ] || t="$FALLBACK_XRAY"
  printf '%s' "$t"
}

# ---------------------------------------------------------------- 安装步骤
step_deps() {
  log "[1/7] 环境与依赖"
  detect_os
  ok "系统 ${OS_ID} ${OS_VER:-?} · ${PKG} · ${INIT}"
  pkg_install nginx curl openssl unzip ca-certificates socat iproute2 || die "依赖安装失败"
  if [ "$PKG" = apk ]; then
    pkg_install gcompat >/dev/null 2>&1 || true
    rc-update add crond default >/dev/null 2>&1 || true
    rc-service crond start >/dev/null 2>&1 || true
  fi
  mkdir -p "$LOG_DIR" "$CONF_DIR" "$SITE_ROOT" "${ACME_WEBROOT}/.well-known/acme-challenge" /etc/nginx/ssl/"$DOMAIN"
}

step_nginx() {
  log "[2/7] 站点与 Nginx"
  tpl_index > "${SITE_ROOT}/index.html"

  local v listen_http2 http2_on
  v=$(nginx -v 2>&1 | sed -n 's|.*nginx/||p' | cut -d- -f1)
  if ver_ge "${v:-0}" 1.25.1; then
    listen_http2=""; http2_on="    http2 on;"
  else
    listen_http2=" http2"; http2_on="    # http2 已写在 listen 行"
  fi

  mkdir -p "$NGINX_SITE_DIR"
  tpl_nginx "$listen_http2" "$http2_on" > "${NGINX_SITE_DIR}/${DOMAIN}.conf"

  if [ "$PKG" = apk ]; then
    rm -f /etc/nginx/http.d/default.conf
  else
    mkdir -p /etc/nginx/sites-enabled
    rm -f /etc/nginx/sites-enabled/default
    ln -sf "${NGINX_SITE_DIR}/${DOMAIN}.conf" "/etc/nginx/sites-enabled/${DOMAIN}.conf"
  fi

  if [ ! -s "/etc/nginx/ssl/${DOMAIN}/fullchain.pem" ]; then
    openssl req -x509 -nodes -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -days 2 \
      -keyout "/etc/nginx/ssl/${DOMAIN}/privkey.pem" \
      -out    "/etc/nginx/ssl/${DOMAIN}/fullchain.pem" \
      -subj "/CN=${DOMAIN}" >/dev/null 2>&1 || die "临时证书生成失败"
  fi

  chown "$NGINX_USER:$NGINX_USER" "$SITE_ROOT" "$ACME_WEBROOT" >/dev/null 2>&1 || true
  nginx -t || die "nginx 配置校验失败"
  svc_start nginx
  [ "$PKG" = dnf ] && have restorecon && restorecon -R /var/www /etc/nginx >/dev/null 2>&1
  ok "Nginx 已起，443 空出来了"
}

step_cert() {
  if [ "$DO_CERT" -eq 0 ]; then warn "[3/7] 按 --no-cert 跳过证书"; return 0; fi
  log "[3/7] 证书（acme.sh）"

  if [ -s "/etc/nginx/ssl/${DOMAIN}/fullchain.pem" ] && \
     openssl x509 -checkend 2592000 -noout -in "/etc/nginx/ssl/${DOMAIN}/fullchain.pem" >/dev/null 2>&1 && \
     openssl x509 -noout -text -in "/etc/nginx/ssl/${DOMAIN}/fullchain.pem" 2>/dev/null | grep -q "$DOMAIN"; then
    ok "证书仍有效（>30 天），跳过签发"
    return 0
  fi

  [ -x "${ACME_HOME}/acme.sh" ] || curl -fsS https://get.acme.sh | sh -s email="$EMAIL" >/dev/null || die "acme.sh 安装失败"
  ACME="${ACME_HOME}/acme.sh"
  "$ACME" --set-default-ca --server letsencrypt >/dev/null 2>&1

  if [ "$ACME_MODE" = dns_cf ]; then
    export CF_Token="$CF_TOKEN"
    "$ACME" --issue --dns dns_cf -d "$DOMAIN" -k ec-256 --force || die "DNS-01 签发失败"
  else
    if ! "$ACME" --issue -d "$DOMAIN" --webroot "$ACME_WEBROOT" --keylength ec-256 --force; then
      warn "HTTP-01 未通过，切 standalone 重试"
      svc_stop nginx
      "$ACME" --issue -d "$DOMAIN" --standalone --keylength ec-256 --force || die "签发失败"
      svc_start nginx
    fi
  fi

  "$ACME" --install-cert -d "$DOMAIN" --ecc \
    --key-file       "/etc/nginx/ssl/${DOMAIN}/privkey.pem" \
    --fullchain-file "/etc/nginx/ssl/${DOMAIN}/fullchain.pem" \
    --reloadcmd      "nginx -s reload" >/dev/null || die "证书安装失败"
  svc_reload nginx
  ok "证书已就位"
}

step_bin() {
  log "[4/7] Xray 内核"
  local tag asset tmp
  asset=$(xray_asset)
  tag=${XRAY_VERSION:-$(latest_xray_tag)}

  if [ -x "$BIN" ] && "$BIN" version 2>/dev/null | grep -q "${tag#v}"; then
    ok "已存在 ${tag}，跳过下载"
    return 0
  fi

  tmp=$(mktemp -d)
  log "下载 ${tag} / ${asset}"
  curl -fL --retry 3 --max-time 300 -o "${tmp}/x.zip" \
    "https://github.com/XTLS/Xray-core/releases/download/${tag}/${asset}" || die "下载失败"
  unzip -o -q "${tmp}/x.zip" -d "$tmp" || die "解压失败"
  install -m 0755 "${tmp}/xray" "$BIN" || die "安装二进制失败"
  rm -rf "$tmp"
  "$BIN" version >/dev/null 2>&1 || die "二进制无法执行（架构不匹配）"
  ok "Xray ${tag} 就位"
}

gen_keys() {
  UUID=$("$BIN" uuid) || die "uuid 生成失败"
  local keyout
  keyout=$("$BIN" x25519)
  PRIV=$(printf '%s\n' "$keyout" | grep -i private | awk -F': *' '{print $2}' | tr -d ' ')
  PUB=$(printf  '%s\n' "$keyout" | grep -iE 'public|password' | awk -F': *' '{print $2}' | tr -d ' ')
  if [ -z "${PRIV:-}" ] && [ -n "${PUB:-}" ]; then PRIV=$PUB; PUB=""; fi
  [ -n "${PRIV:-}" ] || die "x25519 解析失败：$keyout"
  [ -n "${PUB:-}" ]  || PUB=$("$BIN" x25519 -i "$PRIV" | grep -iE 'public|password' | awk -F': *' '{print $2}' | tr -d ' ')
  [ -n "${PUB:-}" ]  || die "公钥派生失败"
  SHORTID=$(rand_hex 8)
  XPATH="/$(rand_hex 6)"
}

step_config() {
  log "[5/7] 密钥与配置"
  gen_keys
  install -d -m 0755 "$CONF_DIR"
  tpl_xray > "$CONF_FILE" || die "写入配置失败"
  chmod 640 "$CONF_FILE"
  "$BIN" run -test -c "$CONF_FILE" || die "配置校验失败"
  ok "配置写入 ${CONF_FILE}"
}

step_service() {
  log "[6/7] 服务注册"
  case "$INIT" in
    systemd)
      local grp=nogroup
      getent group nogroup >/dev/null 2>&1 || grp=nobody
      chgrp "$grp" "$CONF_FILE" >/dev/null 2>&1 || true
      chown root:"$grp" "$LOG_DIR" >/dev/null 2>&1 || true
      chmod 775 "$LOG_DIR"
      tpl_unit_systemd "$grp" > /etc/systemd/system/xray.service
      chmod 0644 /etc/systemd/system/xray.service
      systemctl daemon-reload
      systemctl enable xray >/dev/null 2>&1
      systemctl restart xray ;;
    openrc)
      tpl_init_openrc > /etc/init.d/xray
      chmod 0755 /etc/init.d/xray
      rc-update add xray default >/dev/null 2>&1 || true
      rc-service xray restart ;;
  esac
  sleep 2
  printf '%s  xray 状态：%s%s\n' "$C_D" "$(svc_state xray)" "$C_0"
  ss -lntp 2>/dev/null | grep -E "(:443|:${TLS_PORT})" || warn "端口未就绪，看 ${LOG_DIR}/error.log"
  ok "服务已启动"
}

step_fw() {
  if [ "$DO_FIREWALL" -eq 0 ]; then warn "[7/7] 按 --no-firewall 跳过"; return 0; fi
  log "[7/7] 防火墙"
  fw_apply
}

save_info() {
  local ip link
  ip=$(public_ip)
  if [ "$NETWORK" = xhttp ]; then
    link="vless://${UUID}@${ip}:443?encryption=none&security=reality&sni=${DOMAIN}&fp=chrome&pbk=${PUB}&sid=${SHORTID}&type=xhttp&path=${XPATH}&mode=auto#${DOMAIN}-xhttp"
  else
    link="vless://${UUID}@${ip}:443?encryption=none&flow=xtls-rprx-vision&security=reality&sni=${DOMAIN}&fp=chrome&pbk=${PUB}&sid=${SHORTID}&type=tcp#${DOMAIN}"
  fi
  umask 077
  cat > "$INFO_FILE" <<INFO
domain=${DOMAIN}
server_ip=${ip}
uuid=${UUID}
public_key=${PUB}
private_key=${PRIV}
short_id=${SHORTID}
network=${NETWORK}
path=${XPATH}
tls_port=${TLS_PORT}
url=${link}
INFO
  chmod 600 "$INFO_FILE"

  printf '\n%s──── 部署完成 ────%s\n' "$C_G" "$C_0"
  printf ' 域名     : %s\n' "$DOMAIN"
  printf ' 协议     : %s\n' "$([ "$NETWORK" = xhttp ] && echo "XHTTP + REALITY" || echo "RAW/TCP + REALITY + Vision")"
  printf ' 服务器   : %s\n' "$ip"
  printf ' UUID     : %s\n' "$UUID"
  printf ' 公钥 pbk  : %s\n' "$PUB"
  printf ' shortId  : %s\n' "$SHORTID"
  [ "$NETWORK" = xhttp ] && printf ' XHTTP path: %s\n' "$XPATH"
  printf ' dest     : 127.0.0.1:%s (Nginx)\n' "$TLS_PORT"
  printf '\n 客户端链接：\n %s\n' "$link"
  printf '\n 凭据：%s\n' "$INFO_FILE"
  printf '%s──────────────────%s\n\n' "$C_G" "$C_0"
}

do_install() {
  need_root
  [ -n "$DOMAIN" ] || die "缺少 --domain"
  case "$DOMAIN" in
    *[!a-zA-Z0-9.-]*) die "域名含非法字符" ;;
    *.*) ;;
    *) die "域名要是完整 FQDN，例如 api.example.com" ;;
  esac
  SITE_ROOT=${SITE_ROOT:-/var/www/${DOMAIN}}
  ACME_MODE=${ACME_MODE:-webroot}

  step_deps
  step_nginx
  step_cert
  step_bin
  step_config
  step_service
  step_fw
  save_info
}

# ---------------------------------------------------------------- 子命令
cmd_info() {
  [ -r "$INFO_FILE" ] || die "未找到 ${INFO_FILE}，先执行 rs install"
  printf '\n%s以下为当前节点凭据（%s）：%s\n\n' "$C_B" "$INFO_FILE" "$C_0"
  while IFS='=' read -r k v; do
    case "$k" in
      private_key) printf ' %-11s %s\n' "$k" "${v:0:8}…（已隐藏，看文件）" ;;
      url)         printf ' %-11s\n %s\n' "$k" "$v" ;;
      *)           printf ' %-11s %s\n' "$k" "$v" ;;
    esac
  done < "$INFO_FILE"
  echo
}

cmd_check() {
  need_root
  detect_os
  local d ip
  if [ -r "$INFO_FILE" ]; then
    d=$(sed -n 's/^domain=//p' "$INFO_FILE" | head -1)
    ip=$(sed -n 's/^server_ip=//p' "$INFO_FILE" | head -1)
  fi
  d=${d:-${DOMAIN:-}}
  [ -n "$d" ] || die "拿不到域名"
  ip=${ip:-$(public_ip)}

  echo "== 1. 端口归属 =="
  ss -lntp 2>/dev/null | grep -E "(:443|:${TLS_PORT})" || warn "443 / ${TLS_PORT} 没有监听"

  echo "== 2. 站点 =="
  curl -sI "https://${d}/" 2>/dev/null | head -3 || warn "站点请求失败"

  echo "== 3. 回落链路（本机 8443） =="
  openssl s_client -connect "127.0.0.1:${TLS_PORT}" -servername "$d" -alpn h2 -tls1_3 </dev/null 2>/dev/null \
    | grep -E 'Protocol|ALPN|subject=' || warn "本机回落握手失败"

  echo "== 4. 外部 443 上的真证书 =="
  openssl s_client -connect "${ip}:443" -servername "$d" -alpn h2 -tls1_3 </dev/null 2>/dev/null \
    | grep -E 'Protocol|ALPN|subject=' || warn "外部 443 握手失败"

  echo "== 5. 配置自检 =="
  [ -x "$BIN" ] && "$BIN" run -test -c "$CONF_FILE" || warn "配置校验未通过"

  echo "== 6. 服务 =="
  printf ' xray: %s\n nginx: %s\n' "$(svc_state xray)" "$(svc_state nginx)"
  echo
}

cmd_upgrade() {
  need_root
  detect_os
  [ -x "$BIN" ] || die "Xray 未安装"
  XRAY_VERSION=""
  local old new
  old=$("$BIN" version 2>/dev/null | head -1)
  rm -f "$BIN"
  step_bin
  new=$("$BIN" version 2>/dev/null | head -1)
  "$BIN" run -test -c "$CONF_FILE" || die "新内核配置校验失败"
  svc_restart xray
  ok "内核升级：${old}  ->  ${new}"
  svc_state xray
}

cmd_uninstall() {
  need_root
  detect_os
  local all=${1:-}
  svc_stop xray
  rm -f /etc/systemd/system/xray.service /etc/init.d/xray
  have systemctl && systemctl daemon-reload >/dev/null 2>&1
  rm -f "$BIN" "$INFO_FILE"
  rm -rf "$CONF_DIR" "$LOG_DIR"
  ok "节点已移除"

  if [ -n "${DOMAIN:-}" ] || [ -r /root/.rs.last.domain ]; then
    local d=${DOMAIN:-$(cat /root/.rs.last.domain 2>/dev/null)}
    if [ -n "$d" ]; then
      rm -f "${NGINX_SITE_DIR}/${d}.conf" "/etc/nginx/sites-enabled/${d}.conf"
      nginx -t >/dev/null 2>&1 && svc_reload nginx
      ok "站点 ${d} 已移除"
    fi
  fi

  if [ "$all" = "--all" ]; then
    local d
    d=${DOMAIN:-$(cat /root/.rs.last.domain 2>/dev/null)}
    [ -n "${d:-}" ] && rm -rf "/etc/nginx/ssl/${d}" "/var/www/${d}" "$ACME_WEBROOT"
    [ -x "${ACME_HOME}/acme.sh" ] && [ -n "${d:-}" ] && "${ACME_HOME}/acme.sh" --remove -d "$d" --ecc >/dev/null 2>&1
    fw_remove
    ok "证书 / 站点 / 防火墙已回滚"
  fi
}

cmd_self_update() {
  need_root
  local target
  target=$(command -v rs 2>/dev/null || echo /usr/local/bin/rs)
  log "从 ${RAW_BASE}/rs 拉取最新版本"
  curl -LfsS --max-time 60 "${RAW_BASE}/rs" -o "${target}.new" || die "下载失败"
  bash -n "${target}.new" || { rm -f "${target}.new"; die "新版本语法校验失败，已放弃"; }
  chmod +x "${target}.new"
  mv "${target}.new" "$target"
  ok "已更新 $(basename "$target")（原文件已覆盖）"
}

# ---------------------------------------------------------------- 菜单
ask() { # ask <变量名> <提示> <默认>
  local __v=$1 __p=$2 __d=${3:-} __in=""
  if [ -n "$__d" ]; then printf '%s [%s]: ' "$__p" "$__d"; else printf '%s: ' "$__p"; fi
  read -r __in
  [ -z "$__in" ] && __in=$__d
  printf -v "$__v" '%s' "$__in"
}

menu() {
  need_root
  detect_os
  while :; do
    printf '\n%s┌─ rs %s · REALITY 偷自己 ─────────────%s\n' "$C_B" "$RS_VERSION" "$C_0"
    printf '│ 1) 安装 / 重装   RAW + Vision（推荐）\n'
    printf '│ 2) 安装 / 重装   XHTTP + REALITY\n'
    printf '│ 3) 查看节点信息与链接\n'
    printf '│ 4) 自检\n'
    printf '│ 5) 升级 Xray 内核\n'
    printf '│ 6) 卸载\n'
    printf '│ 7) 更新本脚本\n'
    printf '│ 0) 退出\n'
    printf '%s└──────────────────────────────────────%s\n' "$C_B" "$C_0"
    printf '选择: '
    local c=""
    read -r c || exit 0
    case "$c" in
      1|2)
        NETWORK=raw; [ "$c" = 2 ] && NETWORK=xhttp
        ask DOMAIN "域名（已解析到本机）" "$(cat /root/.rs.last.domain 2>/dev/null)"
        ask EMAIL "邮箱（acme 注册）" "admin@${DOMAIN}"
        ask TLS_PORT "Nginx 回环端口" "$TLS_PORT"
        ask SSH_PORT "SSH 端口" "$SSH_PORT"
        ask CF_TOKEN "Cloudflare Token（留空走 HTTP-01）" ""
        [ -n "$CF_TOKEN" ] && ACME_MODE=dns_cf || ACME_MODE=webroot
        echo "$DOMAIN" > /root/.rs.last.domain
        do_install ;;
      3) cmd_info ;;
      4) cmd_check ;;
      5) cmd_upgrade ;;
      6) ask yn "连站点/证书/防火墙一起清（y/N）" "N"
         if [ "$yn" = y ]; then DOMAIN=$(cat /root/.rs.last.domain 2>/dev/null); cmd_uninstall --all; else cmd_uninstall; fi ;;
      7) cmd_self_update ;;
      0|q|exit) exit 0 ;;
      *) warn "没有这个选项：$c" ;;
    esac
  done
}

usage() {
  cat <<'USAGE'
rs 1.0.0 —— REALITY「偷自己」单文件安装器

用法：
  rs                                  交互菜单
  rs install --domain <fqdn> [选项]    直接安装
  rs info                             查看凭据与客户端链接
  rs check                            自检（端口/回落/证书/服务）
  rs upgrade                          升级 Xray 内核
  rs uninstall [--all]                卸载（--all 连站点/证书/防火墙）
  rs self-update                      重新下载本脚本

install 选项：
  --domain <fqdn>        已解析到本机的域名（必填）
  --email <mail>         acme.sh 注册邮箱，默认 admin@<domain>
  --network <raw|xhttp>  传输层，默认 raw（VLESS + Vision）
  --tls-port <port>      Nginx 回环端口，默认 8443
  --ssh-port <port>      SSH 端口，默认 22
  --cf-token <token>     Cloudflare Token，填了改走 DNS-01
  --xray-version <tag>   指定 Xray 版本，默认最新
  --no-cert              跳过证书申请
  --no-firewall          跳过防火墙
  -y, --yes              无人值守

单行安装：
  (curl -LfsS https://raw.githubusercontent.com/lrexionclub/reality-selfsteal/main/rs \
     -o /usr/local/bin/rs || wget -q https://raw.githubusercontent.com/lrexionclub/reality-selfsteal/main/rs \
     -O /usr/local/bin/rs) && chmod +x /usr/local/bin/rs && rs
USAGE
}

# ---------------------------------------------------------------- 入口
SUB=${1:-}
[ "$#" -gt 0 ] && shift

case "$SUB" in
  "" )                 menu ;;
  install)
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --domain)   DOMAIN=${2:?}; shift 2 ;;
        --email)    EMAIL=${2:?}; shift 2 ;;
        --network)  NETWORK=${2:?}; shift 2 ;;
        --tls-port) TLS_PORT=${2:?}; shift 2 ;;
        --ssh-port) SSH_PORT=${2:?}; shift 2 ;;
        --cf-token) CF_TOKEN=${2:?}; ACME_MODE=dns_cf; shift 2 ;;
        --xray-version) XRAY_VERSION=${2:?}; shift 2 ;;
        --no-cert)  DO_CERT=0; shift ;;
        --no-firewall) DO_FIREWALL=0; shift ;;
        -y|--yes)   ASSUME_YES=1; shift ;;
        *) die "未知参数：$1" ;;
      esac
    done
    case "$NETWORK" in raw|xhttp) ;; *) die "--network 只能 raw 或 xhttp" ;; esac
    [ -n "$DOMAIN" ] || die "缺少 --domain"
    case "$DOMAIN" in
      *[!a-zA-Z0-9.-]*) die "域名含非法字符：$DOMAIN" ;;
      *.*) ;;
      *) die "域名要是完整 FQDN，例如 api.example.com" ;;
    esac
    EMAIL=${EMAIL:-admin@${DOMAIN}}
    if [ -d /root ]; then printf '%s\n' "$DOMAIN" > /root/.rs.last.domain 2>/dev/null || true; fi
    do_install ;;
  info)        cmd_info ;;
  check)       cmd_check ;;
  upgrade)     cmd_upgrade ;;
  uninstall)   cmd_uninstall "${1:-}" ;;
  self-update) cmd_self_update ;;
  install-raw)   NETWORK=raw;   do_install ;;
  install-xhttp) NETWORK=xhttp; do_install ;;
  -h|--help|help) usage ;;
  version|-v) echo "rs ${RS_VERSION} (REALITY self-steal)" ;;
  *) die "未知子命令：${SUB}（rs --help）" ;;
esac
