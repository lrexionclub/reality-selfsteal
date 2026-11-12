#!/usr/bin/env bash
# =============================================================================
#  rs v2.0.0 —— REALITY「偷自己」节点管理器（单文件）
#
#  结构：Nginx 退回 127.0.0.1:8443（真域名/真证书/真站点），
#        Xray 独占 0.0.0.0:443，REALITY dest 指向本机 Nginx。
#
#  支持：Alpine(apk/OpenRC/nftables) · Debian/Ubuntu(apt/systemd/ufw) · RHEL(dnf/systemd/firewalld)
#
#  安装：
#   (curl -LfsS https://raw.githubusercontent.com/lrexionclub/reality-selfsteal/main/rs \
#      -o /usr/local/bin/rs || wget -q https://raw.githubusercontent.com/lrexionclub/reality-selfsteal/main/rs \
#      -O /usr/local/bin/rs) && chmod +x /usr/local/bin/rs && rs
#
#  以后直接运行：  rs
#
#  菜单框架与状态管理思路改写自 0xdabiaoge/singbox-lite（sing-box + Xray 多协议脚本）；
#  REALITY / 偷自己 / 回落链路相关实现为本项目自有。
# =============================================================================

if [ -z "${BASH_VERSION:-}" ]; then
  if command -v apk >/dev/null 2>&1; then
    apk add --no-cache bash >/dev/null 2>&1 || true
  fi
  if command -v bash >/dev/null 2>&1; then
    exec bash "$0" "$@"
  fi
  printf '[xx] 需要 bash，请先 apk add bash\n' >&2
  exit 1
fi

set -uo pipefail
umask 077

RS_VERSION="2.0.0"
REPO_SLUG="${RS_REPO_SLUG:-lrexionclub/reality-selfsteal}"
RAW_BASE="https://raw.githubusercontent.com/${REPO_SLUG}/main"
FALLBACK_XRAY="v26.11.9"

BIN=/usr/local/bin/xray
CONF_DIR=/usr/local/etc/xray
CONF_FILE=${CONF_DIR}/config.json
LOG_DIR=/var/log/xray
STATE_DIR=/usr/local/etc/rs
STATE_FILE=${STATE_DIR}/state.conf
CLIENTS_FILE=${STATE_DIR}/clients.conf
LOCK_FILE=${STATE_DIR}/version.lock
EXPORT_DIR=${STATE_DIR}/export
LEGACY_INFO=/root/reality-selfsteal.info
ACME_HOME=/root/.acme.sh
ACME_WEBROOT=/var/www/acme

DOMAIN=""
TLS_PORT=8443
NETWORK="raw"
XPATH=""
SHORTID=""
PRIV=""
PUB=""
SERVER_ADDR=""
SSH_PORT=22
EMAIL=""
CF_TOKEN=""
ACME_MODE="webroot"
DO_CERT=1
DO_FIREWALL=1
XRAY_VERSION=""
ASSUME_YES=0
PKG=""
INIT=""
NGINX_USER=""
NGINX_SITE_DIR=""

if [ -t 1 ]; then
  C_R=$'\033[31m'; C_G=$'\033[32m'; C_Y=$'\033[33m'; C_B=$'\033[36m'; C_0=$'\033[0m'
else
  C_R=""; C_G=""; C_Y=""; C_B=""; C_0=""
fi
log()  { printf '%s[rs]%s %s\n' "$C_B" "$C_0" "$*"; }
ok()   { printf '%s[ok]%s %s\n' "$C_G" "$C_0" "$*"; }
warn() { printf '%s[!!]%s %s\n' "$C_Y" "$C_0" "$*" >&2; }
die()  { printf '%s[xx]%s %s\n' "$C_R" "$C_0" "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }
need_root() { [ "$(id -u)" -eq 0 ] || die "需要 root 权限"; }

# =============================================================================
# 发行版 / 包管理 / 服务 / 防火墙
# =============================================================================
detect_os() {
  local os_id
  os_id=unknown; OS_VER=""
  if [ -r /etc/os-release ]; then
    # shellcheck disable=SC1091
    . /etc/os-release
    os_id=${ID:-unknown}; OS_VER=${VERSION_ID:-}
  fi
  OS_ID=$os_id
  case "$os_id" in
    alpine)                                   PKG=apk ;;
    debian|ubuntu|raspbian|armbian|linuxmint) PKG=apt ;;
    centos|rhel|rocky|almalinux|fedora|ol)    PKG=dnf ;;
    *)
      if   have apk;     then PKG=apk
      elif have apt-get; then PKG=apt
      elif have dnf;     then PKG=dnf
      else die "未识别的发行版：${os_id}"
      fi ;;
  esac
  INIT=unknown
  if have systemctl && [ -d /run/systemd/system ]; then INIT=systemd; fi
  if [ -x /sbin/rc-service ] || [ -f /sbin/openrc-run ]; then INIT=openrc; fi
  case "$PKG" in
    apk) NGINX_USER=nginx;    NGINX_SITE_DIR=/etc/nginx/http.d ;;
    *)   NGINX_USER=www-data; NGINX_SITE_DIR=/etc/nginx/sites-available ;;
  esac
  export OS_ID OS_VER PKG INIT NGINX_USER NGINX_SITE_DIR
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

svc_enable() { case "$INIT" in
    systemd) systemctl enable "$1" >/dev/null 2>&1 || true ;;
    openrc)  rc-update add "$1" default >/dev/null 2>&1 || true ;;
    *)       : ;;
  esac; }
svc_start() { case "$INIT" in
    systemd) systemctl start "$1" ;;
    openrc)  rc-service "$1" start ;;
    *)       die "未知 init，无法启动 ${1}" ;;
  esac; }
svc_stop() { case "$INIT" in
    systemd) systemctl stop "$1" 2>/dev/null || true ;;
    openrc)  rc-service "$1" stop 2>/dev/null || true ;;
    *)       : ;;
  esac; }
svc_restart() { case "$INIT" in
    systemd) systemctl restart "$1" ;;
    openrc)  rc-service "$1" restart ;;
    *)       die "未知 init，无法重启 ${1}" ;;
  esac; }
svc_reload() { case "$INIT" in
    systemd) systemctl reload "$1" 2>/dev/null || systemctl restart "$1" ;;
    openrc)  rc-service "$1" reload 2>/dev/null || rc-service "$1" restart ;;
    *)       die "未知 init，无法重载 ${1}" ;;
  esac; }
svc_state() { case "$INIT" in
    systemd) systemctl is-active "$1" 2>/dev/null ;;
    openrc)  rc-service "$1" status 2>/dev/null | head -1 ;;
    *)       printf 'unknown\n' ;;
  esac; }

fw_apply() {
  local ssh_port=${SSH_PORT:-22}
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
        tcp dport { ${ssh_port}, 80, 443 } accept
    }
    chain forward { type filter hook forward priority 0; policy drop; }
    chain output  { type filter hook output  priority 0; policy accept; }
}
NFT
      nft -c -f /etc/nftables.nft || die "nftables 规则校验失败，未应用"
      rc-update add nftables default >/dev/null 2>&1 || true
      rc-service nftables restart || die "nftables 加载失败"
      ok "nftables：放行 ${ssh_port}/80/443，其余入站拒绝" ;;
    apt)
      pkg_install ufw || die "ufw 安装失败"
      ufw --force reset >/dev/null 2>&1 || true
      ufw default deny incoming >/dev/null
      ufw default allow outgoing >/dev/null
      ufw allow "${ssh_port}/tcp" >/dev/null
      ufw allow 80/tcp >/dev/null
      ufw allow 443/tcp >/dev/null
      ufw --force enable >/dev/null
      ok "ufw：放行 ${ssh_port}/80/443" ;;
    dnf)
      pkg_install firewalld || die "firewalld 安装失败"
      systemctl enable --now firewalld >/dev/null 2>&1 || true
      firewall-cmd --permanent --add-port="${ssh_port}/tcp" >/dev/null
      firewall-cmd --permanent --add-service=http  >/dev/null
      firewall-cmd --permanent --add-service=https >/dev/null
      firewall-cmd --reload >/dev/null
      ok "firewalld：放行 ${ssh_port}/80/443" ;;
  esac
}

# =============================================================================
# 状态
# =============================================================================
state_init() {
  mkdir -p "$STATE_DIR" "$EXPORT_DIR"
  chmod 700 "$STATE_DIR"
  [ -f "$STATE_FILE" ]   || : > "$STATE_FILE"
  [ -f "$CLIENTS_FILE" ] || : > "$CLIENTS_FILE"
  chmod 600 "$STATE_FILE" "$CLIENTS_FILE"
}

state_load() {
  [ -f "$STATE_FILE" ] || return 1
  local k v
  while IFS='=' read -r k v; do
    case "$k" in
      DOMAIN)      DOMAIN=$v ;;
      TLS_PORT)    TLS_PORT=$v ;;
      NETWORK)     NETWORK=$v ;;
      XPATH)       XPATH=$v ;;
      SHORTID)     SHORTID=$v ;;
      PRIV)        PRIV=$v ;;
      PUB)         PUB=$v ;;
      SERVER_ADDR) SERVER_ADDR=$v ;;
      *)           : ;;
    esac
  done < "$STATE_FILE"
  [ -n "$DOMAIN" ] && [ -n "$PRIV" ]
}

state_save() {
  state_init
  {
    printf 'DOMAIN=%s\n'      "$DOMAIN"
    printf 'TLS_PORT=%s\n'    "$TLS_PORT"
    printf 'NETWORK=%s\n'     "$NETWORK"
    printf 'XPATH=%s\n'       "$XPATH"
    printf 'SHORTID=%s\n'     "$SHORTID"
    printf 'PRIV=%s\n'        "$PRIV"
    printf 'PUB=%s\n'         "$PUB"
    printf 'SERVER_ADDR=%s\n' "$SERVER_ADDR"
  } > "$STATE_FILE"
  chmod 600 "$STATE_FILE"
}

legacy_import() {
  [ -s "$STATE_FILE" ] && return 0
  [ -r "$LEGACY_INFO" ] || return 0
  local k v domain="" tls=8443 net=raw path="" sid="" pk="" qk="" ip="" uuid="" flow=""
  while IFS='=' read -r k v; do
    case "$k" in
      domain)      domain=$v ;;
      tls_port)    tls=$v ;;
      network)     net=$v ;;
      path)        path=$v ;;
      short_id)    sid=$v ;;
      private_key) pk=$v ;;
      public_key)  qk=$v ;;
      server_ip)   ip=$v ;;
      uuid)        uuid=$v ;;
      *)           : ;;
    esac
  done < "$LEGACY_INFO"
  if [ -n "$domain" ] && [ -n "$pk" ]; then
    DOMAIN=$domain; TLS_PORT=$tls; NETWORK=$net; XPATH=$path
    SHORTID=$sid; PRIV=$pk; PUB=$qk; SERVER_ADDR=$ip
    state_save
    [ "$net" = raw ] && flow=xtls-rprx-vision || flow=-
    : > "$CLIENTS_FILE"
    printf '%s|user1@%s|%s|%s\n' "$uuid" "$domain" "$flow" "$(date +%F)" >> "$CLIENTS_FILE"
    chmod 600 "$CLIENTS_FILE"
    ok "已从 ${LEGACY_INFO} 导入旧节点状态"
  fi
}

clients_count() {
  local n
  n=$(grep -c '|' "$CLIENTS_FILE" 2>/dev/null) || n=0
  printf '%s' "${n:-0}"
}

# =============================================================================
# 模板
# =============================================================================
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
  local listen_http2=$1 http2_on=$2 site_root=${3:-/var/www/${DOMAIN}}
  cat <<NGINX
server {
    listen 80;
    listen [::]:80;
    server_name ${DOMAIN};

    location /.well-known/acme-challenge/ { root ${ACME_WEBROOT}; }
    location / { return 301 https://\$host\$request_uri; }
}

server {
    # 127.0.0.1 前缀：只给本机 Xray 回落用，外部扫不到
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

    root ${site_root};
    index index.html;
    location / { try_files \$uri \$uri/ =404; }
    location /.well-known/acme-challenge/ { root ${ACME_WEBROOT}; }
}
NGINX
}

tpl_xray() {
  local uuid email flow first=1 body="" extra xhttp_field="" net
  net=$([ "$NETWORK" = xhttp ] && printf xhttp || printf raw)
  if [ "$NETWORK" = xhttp ]; then
    xhttp_field="        \"xhttpSettings\": {
          \"path\": \"${XPATH}\",
          \"mode\": \"auto\",
          \"extra\": { \"xPaddingBytes\": \"100-1000\", \"scMaxEachPostBytes\": 1000000 }
        },"
    xhttp_field="${xhttp_field}"$'\n'
  fi
  while IFS='|' read -r uuid email flow _; do
    [ -n "${uuid:-}" ] || continue
    extra=""
    if [ "$NETWORK" = raw ] && [ -n "${flow:-}" ] && [ "$flow" != "-" ]; then
      extra=",
            \"flow\": \"${flow}\""
    fi
    if [ "$first" -eq 1 ]; then
      body="          { \"id\": \"${uuid}\", \"email\": \"${email}\"${extra} }"
      first=0
    else
      body="${body},
          { \"id\": \"${uuid}\", \"email\": \"${email}\"${extra} }"
    fi
  done < "$CLIENTS_FILE"

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
${body}
        ],
        "decryption": "none"
      },
      "streamSettings": {
        "network": "${net}",
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

# =============================================================================
# 工具
# =============================================================================
rand_hex() { openssl rand -hex "${1:-8}" 2>/dev/null || od -An -tx1 -N"${1:-8}" /dev/urandom | tr -d ' \n'; }

public_ip() {
  local ip="" u
  for u in https://api.ipify.org https://ifconfig.me/ip https://ipv4.icanhazip.com; do
    ip=$(curl -4 -fsS --max-time 6 "$u" 2>/dev/null | tr -d '[:space:]') || true
    case "$ip" in *[0-9].[0-9]*) printf '%s' "$ip"; return 0 ;; esac
  done
  printf '0.0.0.0'
}

ver_ge() {
  local a=$1 b=$2 i x y
  local -a av bv
  IFS=. read -r -a av <<< "$a"
  IFS=. read -r -a bv <<< "$b"
  for i in 0 1 2; do
    x=${av[i]:-0}; x=${x//[^0-9]/}; x=${x:-0}
    y=${bv[i]:-0}; y=${y//[^0-9]/}; y=${y:-0}
    if [ "$((10#$x))" -gt "$((10#$y))" ]; then return 0; fi
    if [ "$((10#$x))" -lt "$((10#$y))" ]; then return 1; fi
  done
  return 0
}

xray_asset() {
  case "$(uname -m)" in
    x86_64|amd64)  printf 'Xray-linux-64.zip\n' ;;
    aarch64|arm64) printf 'Xray-linux-arm64-v8a.zip\n' ;;
    armv7l|armv7)  printf 'Xray-linux-arm32-v7a.zip\n' ;;
    i386|i686)     printf 'Xray-linux-32.zip\n' ;;
    s390x)         printf 'Xray-linux-s390x.zip\n' ;;
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

link_for() { # link_for <uuid>
  local uuid=$1 addr srv
  if [ "$NETWORK" = xhttp ]; then
    addr="vless://${uuid}@${SERVER_ADDR}:443?encryption=none&security=reality&sni=${DOMAIN}&fp=chrome&pbk=${PUB}&sid=${SHORTID}&type=xhttp&path=${XPATH}&mode=auto"
  else
    addr="vless://${uuid}@${SERVER_ADDR}:443?encryption=none&flow=xtls-rprx-vision&security=reality&sni=${DOMAIN}&fp=chrome&pbk=${PUB}&sid=${SHORTID}&type=tcp"
  fi
  srv=$(printf '%s' "$DOMAIN" | tr '.' '-')
  printf '%s#%s-%s' "$addr" "$srv" "${uuid:0:6}"
}

# =============================================================================
# 渲染 / 生效
# =============================================================================
render_all() {
  local v listen_http2 http2_on
  v=$(nginx -v 2>&1 | sed -n 's|.*nginx/||p' | cut -d- -f1)
  if ver_ge "${v:-0}" 1.25.1; then
    listen_http2=""; http2_on="    http2 on;"
  else
    listen_http2=" http2"; http2_on="    # http2 已写在 listen 行"
  fi
  mkdir -p "$NGINX_SITE_DIR" "$CONF_DIR" "$LOG_DIR"
  tpl_nginx "$listen_http2" "$http2_on" > "${NGINX_SITE_DIR}/${DOMAIN}.conf"
  if [ "$PKG" = apk ]; then
    rm -f /etc/nginx/http.d/default.conf
  else
    mkdir -p /etc/nginx/sites-enabled
    ln -sf "${NGINX_SITE_DIR}/${DOMAIN}.conf" "/etc/nginx/sites-enabled/${DOMAIN}.conf"
  fi
  tpl_xray > "$CONF_FILE"
  chmod 640 "$CONF_FILE"
  nginx -t || die "nginx 配置校验失败"
  "$BIN" run -test -c "$CONF_FILE" || die "Xray 配置校验失败"
}

apply_all() {
  render_all
  if [ "$INIT" = systemd ]; then
    local grp=nogroup
    getent group nogroup >/dev/null 2>&1 || grp=nobody
    chgrp "$grp" "$CONF_FILE" >/dev/null 2>&1 || true
    chown "root:${grp}" "$LOG_DIR" >/dev/null 2>&1 || true
    chmod 775 "$LOG_DIR"
    tpl_unit_systemd "$grp" > /etc/systemd/system/xray.service
    chmod 0644 /etc/systemd/system/xray.service
    systemctl daemon-reload
    svc_enable xray
  else
    tpl_init_openrc > /etc/init.d/xray
    chmod 0755 /etc/init.d/xray
    svc_enable xray
  fi
  svc_reload nginx
  svc_restart xray
  sleep 1
  printf '  xray=%s  nginx=%s\n' "$(svc_state xray)" "$(svc_state nginx)"
}

# =============================================================================
# 安装流程
# =============================================================================
step_deps() {
  log "[1/6] 环境与依赖"
  detect_os
  ok "系统 ${OS_ID} ${OS_VER:-?} · ${PKG} · ${INIT}"
  pkg_install nginx curl openssl unzip ca-certificates socat iproute2 || die "依赖安装失败"
  if [ "$PKG" = apk ]; then
    pkg_install gcompat >/dev/null 2>&1 || true
    rc-update add crond default >/dev/null 2>&1 || true
    rc-service crond start >/dev/null 2>&1 || true
  fi
  mkdir -p "$LOG_DIR" "$CONF_DIR" /var/www/"$DOMAIN" "${ACME_WEBROOT}/.well-known/acme-challenge" /etc/nginx/ssl/"$DOMAIN"
  chown "${NGINX_USER}:${NGINX_USER}" /var/www/"$DOMAIN" "$ACME_WEBROOT" >/dev/null 2>&1 || true
}

step_site() {
  log "[2/6] 站点与 Nginx"
  if [ ! -s "/etc/nginx/ssl/${DOMAIN}/fullchain.pem" ]; then
    openssl req -x509 -nodes -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -days 2 \
      -keyout "/etc/nginx/ssl/${DOMAIN}/privkey.pem" \
      -out    "/etc/nginx/ssl/${DOMAIN}/fullchain.pem" \
      -subj "/CN=${DOMAIN}" >/dev/null 2>&1 || die "临时证书生成失败"
  fi
  tpl_index > /var/www/"$DOMAIN"/index.html
  # 先写好 Xray 配置雏形，让 render_all 校验能过
  : > "$CLIENTS_FILE"
  tpl_xray > "$CONF_FILE"
  chmod 640 "$CONF_FILE"
  mkdir -p "$NGINX_SITE_DIR"
  tpl_nginx "" "" > "${NGINX_SITE_DIR}/${DOMAIN}.conf"
  if [ "$PKG" = apk ]; then
    rm -f /etc/nginx/http.d/default.conf
  else
    mkdir -p /etc/nginx/sites-enabled
    ln -sf "${NGINX_SITE_DIR}/${DOMAIN}.conf" "/etc/nginx/sites-enabled/${DOMAIN}.conf"
  fi
  nginx -t || die "nginx 配置校验失败"
  svc_enable nginx
  svc_restart nginx
  [ "$PKG" = dnf ] && have restorecon && restorecon -R /var/www /etc/nginx >/dev/null 2>&1
  ok "Nginx 已起，443 空出"
}

step_cert() {
  if [ "$DO_CERT" -eq 0 ]; then warn "[3/6] 按 --no-cert 跳过证书"; return 0; fi
  log "[3/6] 证书"
  if [ -s "/etc/nginx/ssl/${DOMAIN}/fullchain.pem" ] && \
     openssl x509 -checkend 2592000 -noout -in "/etc/nginx/ssl/${DOMAIN}/fullchain.pem" >/dev/null 2>&1 && \
     openssl x509 -noout -text -in "/etc/nginx/ssl/${DOMAIN}/fullchain.pem" 2>/dev/null | grep -q "$DOMAIN"; then
    ok "已有有效证书（>30 天），跳过"
    return 0
  fi
  if [ ! -x "${ACME_HOME}/acme.sh" ]; then
    curl -fsS https://get.acme.sh | sh -s email="${EMAIL:-admin@${DOMAIN}}" >/dev/null || die "acme.sh 安装失败"
  fi
  local acme="${ACME_HOME}/acme.sh"
  "$acme" --set-default-ca --server letsencrypt >/dev/null 2>&1
  if [ "$ACME_MODE" = dns_cf ] && [ -n "$CF_TOKEN" ]; then
    export CF_Token="$CF_TOKEN"
    "$acme" --issue --dns dns_cf -d "$DOMAIN" -k ec-256 --force || die "DNS-01 签发失败"
  else
    if ! "$acme" --issue -d "$DOMAIN" --webroot "$ACME_WEBROOT" --keylength ec-256 --force; then
      warn "HTTP-01 未通过，切 standalone 重试"
      svc_stop nginx
      "$acme" --issue -d "$DOMAIN" --standalone --keylength ec-256 --force || die "签发失败"
      svc_start nginx 2>/dev/null || svc_restart nginx
    fi
  fi
  "$acme" --install-cert -d "$DOMAIN" --ecc \
    --key-file       "/etc/nginx/ssl/${DOMAIN}/privkey.pem" \
    --fullchain-file "/etc/nginx/ssl/${DOMAIN}/fullchain.pem" \
    --reloadcmd      "nginx -s reload" >/dev/null || die "证书安装失败"
  svc_reload nginx
  ok "证书已就位"
}

step_bin() {
  log "[4/6] Xray 内核"
  local tag asset tmp
  asset=$(xray_asset)
  if [ -f "$LOCK_FILE" ]; then
    tag=$(cat "$LOCK_FILE")
    ok "内核版本已锁定：${tag}"
  else
    tag=${XRAY_VERSION:-$(latest_xray_tag)}
  fi
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

gen_keys() { # 输出 uuid，副作用：刷新 PRIV/PUB
  local uuid keyout
  uuid=$("$BIN" uuid) || die "uuid 生成失败"
  keyout=$("$BIN" x25519)
  PRIV=$(printf '%s\n' "$keyout" | grep -i private | awk -F': *' '{print $2}' | tr -d ' ')
  PUB=$(printf  '%s\n' "$keyout" | grep -iE 'public|password' | awk -F': *' '{print $2}' | tr -d ' ')
  if [ -z "${PRIV:-}" ] && [ -n "${PUB:-}" ]; then PRIV=$PUB; PUB=""; fi
  [ -n "${PRIV:-}" ] || die "x25519 解析失败"
  if [ -z "${PUB:-}" ]; then
    PUB=$("$BIN" x25519 -i "$PRIV" | grep -iE 'public|password' | awk -F': *' '{print $2}' | tr -d ' ')
  fi
  [ -n "${PUB:-}" ] || die "公钥派生失败"
  printf '%s' "$uuid"
}

save_client() { # save_client <uuid> <email>
  state_init
  local flow="-"
  [ "$NETWORK" = raw ] && flow=xtls-rprx-vision
  printf '%s|%s|%s|%s\n' "$1" "$2" "$flow" "$(date +%F)" >> "$CLIENTS_FILE"
  chmod 600 "$CLIENTS_FILE"
}

do_install() {
  [ -n "$DOMAIN" ] || die "缺少 --domain"
  case "$DOMAIN" in
    *[!a-zA-Z0-9.-]*) die "域名含非法字符：$DOMAIN" ;;
    *.*) ;;
    *) die "域名要是完整 FQDN，例如 api.example.com" ;;
  esac
  need_root
  EMAIL=${EMAIL:-admin@${DOMAIN}}
  SERVER_ADDR=${SERVER_ADDR:-$(public_ip)}
  state_init
  step_deps
  [ -n "${SHORTID:-}" ] || SHORTID=$(rand_hex 8)
  [ -n "${XPATH:-}" ]   || XPATH="/$(rand_hex 6)"
  step_site
  step_cert
  step_bin
  local uuid
  uuid=$(gen_keys)
  state_save
  : > "$CLIENTS_FILE"
  save_client "$uuid" "user1@${DOMAIN}"
  log "[5/6] 写入配置"
  apply_all
  if [ "$DO_FIREWALL" -eq 1 ]; then
    log "[6/6] 防火墙"
    fw_apply
  fi
  export_links >/dev/null
  show_summary
}

# =============================================================================
# 展示 / 导出
# =============================================================================
show_summary() {
  local uuid email flow
  printf '\n%s──── 节点信息 ────%s\n' "$C_G" "$C_0"
  printf ' 域名     : %s\n' "$DOMAIN"
  if [ "$NETWORK" = xhttp ]; then
    printf ' 协议     : XHTTP + REALITY\n'
  else
    printf ' 协议     : RAW/TCP + REALITY + Vision\n'
  fi
  printf ' 服务器   : %s\n' "$SERVER_ADDR"
  printf ' 回环端口 : %s\n shortId  : %s\n' "$TLS_PORT" "$SHORTID"
  printf ' 公钥 pbk : %s\n' "$PUB"
  if [ "$NETWORK" = xhttp ]; then printf ' XHTTP路径: %s\n' "$XPATH"; fi
  printf ' 客户端   : %s 个\n' "$(clients_count)"
  printf '%s──────────────────%s\n' "$C_G" "$C_0"
  while IFS='|' read -r uuid email flow _; do
    [ -n "${uuid:-}" ] || continue
    printf '\n[%s]\n%s\n' "$email" "$(link_for "$uuid")"
  done < "$CLIENTS_FILE"
  printf '\n状态目录：%s/\n' "$STATE_DIR"
  if [ "$ASSUME_YES" -eq 0 ] && [ -t 0 ]; then
    printf '按回车继续… '
    read -r _ || true
  fi
}

export_links() {
  need_root
  state_load || die "尚未安装节点"
  state_init
  SERVER_ADDR=${SERVER_ADDR:-$(public_ip)}
  local uuid email flow total=0 net_out=tcp
  [ "$NETWORK" = xhttp ] && net_out=xhttp
  : > "${EXPORT_DIR}/links.txt"
  printf '# %s  %s\n' "$DOMAIN" "$(date '+%F %T')" >> "${EXPORT_DIR}/links.txt"
  while IFS='|' read -r uuid email flow _; do
    [ -n "${uuid:-}" ] || continue
    printf '%s\n' "$(link_for "$uuid")" >> "${EXPORT_DIR}/links.txt"
    total=$((total + 1))
  done < "$CLIENTS_FILE"
  grep -v '^#' "${EXPORT_DIR}/links.txt" | base64 | tr -d '\n' > "${EXPORT_DIR}/sub.txt"
  {
    printf 'proxies:\n'
    while IFS='|' read -r uuid email flow _; do
      [ -n "${uuid:-}" ] || continue
      printf '  - name: "%s-%s"\n' "$DOMAIN" "${uuid:0:6}"
      printf '    type: vless\n    server: %s\n    port: 443\n    uuid: %s\n    network: %s\n' \
        "$SERVER_ADDR" "$uuid" "$net_out"
      printf '    tls: true\n    udp: true\n    servername: %s\n    client-fingerprint: chrome\n' "$DOMAIN"
      printf '    reality-opts:\n      public-key: %s\n      short-id: "%s"\n' "$PUB" "$SHORTID"
      if [ "$NETWORK" = xhttp ]; then
        printf '    xhttp-opts:\n      path: %s\n      mode: auto\n' "$XPATH"
      else
        printf '    flow: xtls-rprx-vision\n'
      fi
    done < "$CLIENTS_FILE"
    printf 'proxy-groups:\n  - name: PROXY\n    type: select\n    proxies:\n'
    while IFS='|' read -r uuid _ _ _; do
      [ -n "${uuid:-}" ] || continue
      printf '      - "%s-%s"\n' "$DOMAIN" "${uuid:0:6}"
    done < "$CLIENTS_FILE"
    printf 'rules:\n  - MATCH,PROXY\n'
  } > "${EXPORT_DIR}/clash.yaml"
  chmod 600 "${EXPORT_DIR}"/* 2>/dev/null || true
  ok "已导出 ${total} 条链接"
  printf '  链接清单   : %s\n  订阅(base64): %s\n  Clash/Mihomo: %s\n' \
    "${EXPORT_DIR}/links.txt" "${EXPORT_DIR}/sub.txt" "${EXPORT_DIR}/clash.yaml"
}

# =============================================================================
# 交互动作
# =============================================================================
askv() { # askv <变量名> <提示> <默认>
  local __v=$1 __p=$2 __d=${3:-} __in=""
  if [ -n "$__d" ]; then printf '%s [%s]: ' "$__p" "$__d"; else printf '%s: ' "$__p"; fi
  read -r __in || __in=""
  [ -z "$__in" ] && __in=$__d
  printf -v "$__v" '%s' "$__in"
}

act_install() {
  askv pick "传输层 1=raw+Vision  2=xhttp" "1"
  if [ "$pick" = 2 ]; then NETWORK=xhttp; else NETWORK=raw; fi
  local last=""
  [ -f "$STATE_FILE" ] && last=$(sed -n 's/^DOMAIN=//p' "$STATE_FILE" | head -1)
  askv DOMAIN "域名（已解析到本机）" "$last"
  askv EMAIL  "邮箱（acme 注册）" "admin@${DOMAIN}"
  askv TLS_PORT "Nginx 回环端口" "8443"
  askv SSH_PORT "SSH 端口" "22"
  askv CF_TOKEN "Cloudflare Token（留空走 HTTP-01）" ""
  if [ -n "$CF_TOKEN" ]; then ACME_MODE=dns_cf; else ACME_MODE=webroot; fi
  do_install
}

act_add_client() {
  need_root
  state_load || die "尚未安装，请先执行 [1] 安装节点"
  state_init
  local n email uuid i=0
  askv n "添加几个客户端" "1"
  case "$n" in ''|*[!0-9]*) die "数量要是数字" ;; esac
  while [ "$i" -lt "$n" ]; do
    i=$((i + 1))
    askv email "第 ${i} 个备注（生成 email）" "user$(( $(clients_count) + 1 ))@${DOMAIN}"
    uuid=$("$BIN" uuid)
    save_client "$uuid" "$email"
    state_save
    printf '[%s]\n%s\n' "$email" "$(link_for "$uuid")"
  done
  apply_all >/dev/null
  export_links >/dev/null
  ok "已添加 ${n} 个客户端，当前共 $(clients_count) 个"
}

act_list() {
  need_root
  state_load || die "尚未安装"
  SERVER_ADDR=${SERVER_ADDR:-$(public_ip)}
  show_summary
}

act_del_client() {
  need_root
  state_load || die "尚未安装"
  local uuid email flow idx=0 pick="" total
  printf '%-3s %-38s %s\n' '#' 'UUID' 'EMAIL'
  while IFS='|' read -r uuid email flow _; do
    [ -n "${uuid:-}" ] || continue
    idx=$((idx + 1))
    printf '%-3s %-38s %s\n' "$idx" "$uuid" "$email"
  done < "$CLIENTS_FILE"
  askv pick "要删除的编号（0=取消）" "0"
  case "$pick" in ''|0|*[!0-9]*) ok "已取消"; return 0 ;; esac
  total=$(clients_count)
  if [ "$pick" -gt "$total" ]; then die "编号超出范围"; fi
  if [ "$total" -le 1 ]; then die "只剩一个客户端，先加新的再删"; fi
  awk -F'|' -v n="$pick" 'NR!=n' "$CLIENTS_FILE" > "${CLIENTS_FILE}.tmp"
  mv "${CLIENTS_FILE}.tmp" "$CLIENTS_FILE"
  chmod 600 "$CLIENTS_FILE"
  apply_all >/dev/null
  export_links >/dev/null
  ok "已删除第 ${pick} 个客户端，配置已重载"
}

act_edit() {
  need_root
  state_load || die "尚未安装"
  printf '当前：域名=%s 网络=%s 回环=%s path=%s\n' "$DOMAIN" "$NETWORK" "$TLS_PORT" "${XPATH:--}"
  local old_domain=$DOMAIN
  askv DOMAIN "新域名（回车保持）" "$DOMAIN"
  askv NETWORK "网络 raw/xhttp（回车保持）" "$NETWORK"
  askv TLS_PORT "回环端口（回车保持）" "$TLS_PORT"
  askv SERVER_ADDR "客户端连接地址（回车保持）" "$SERVER_ADDR"
  case "$NETWORK" in raw|xhttp) ;; *) die "网络只能是 raw 或 xhttp" ;; esac
  if [ "$NETWORK" = xhttp ] && [ -z "${XPATH:-}" ]; then XPATH="/$(rand_hex 6)"; fi
  local uuid email flow
  while IFS='|' read -r uuid email flow _; do
    [ -n "${uuid:-}" ] || continue
    if [ "$NETWORK" = raw ]; then flow=xtls-rprx-vision; else flow=-; fi
    printf '%s|%s|%s|%s\n' "$uuid" "$email" "$flow" "$(date +%F)"
  done < "$CLIENTS_FILE" > "${CLIENTS_FILE}.tmp"
  mv "${CLIENTS_FILE}.tmp" "$CLIENTS_FILE"
  chmod 600 "$CLIENTS_FILE"
  state_save
  if [ "$DOMAIN" != "$old_domain" ]; then
    warn "域名已变更，证书需要重签"
    mkdir -p /etc/nginx/ssl/"$DOMAIN"
    rm -f /etc/nginx/ssl/"$DOMAIN"/fullchain.pem /etc/nginx/ssl/"$DOMAIN"/privkey.pem
    step_cert
  fi
  apply_all
  export_links >/dev/null
  ok "已更新并重载"
}

act_rotate() {
  need_root
  state_load || die "尚未安装"
  local yn uuid
  askv yn "轮换 REALITY 密钥会作废全部旧链接，继续？(y/N)" "N"
  if [ "$yn" != y ]; then ok "已取消"; return 0; fi
  gen_keys >/dev/null
  state_save
  apply_all >/dev/null
  export_links >/dev/null
  ok "密钥已轮换，pubkey=${PUB}"
  act_list
}

# =============================================================================
# 服务 / 日志 / 定时
# =============================================================================
act_restart() { need_root; svc_restart xray; sleep 1; printf 'xray: %s\n' "$(svc_state xray)"; }
act_stop()    { need_root; svc_stop xray; printf 'xray: %s\n' "$(svc_state xray)"; }
act_start()   { need_root; svc_start xray; sleep 1; printf 'xray: %s\n' "$(svc_state xray)"; }

act_status() {
  need_root
  state_load >/dev/null 2>&1 || true
  printf 'xray : %s\nnginx: %s\n' "$(svc_state xray)" "$(svc_state nginx)"
  ss -lntp 2>/dev/null | grep -E "(:443|:${TLS_PORT})" || warn "没看到 443 / 回环端口监听"
  if [ -x "$BIN" ]; then printf '\n内核：%s\n' "$("$BIN" version 2>/dev/null | head -1)"; fi
  if [ -n "${DOMAIN:-}" ] && [ -s "/etc/nginx/ssl/${DOMAIN}/fullchain.pem" ]; then
    printf '证书到期：%s\n' "$(openssl x509 -noout -enddate -in "/etc/nginx/ssl/${DOMAIN}/fullchain.pem" 2>/dev/null | cut -d= -f2)"
  fi
}

act_log() {
  need_root
  local n
  askv n "看最后多少行" "60"
  case "$n" in ''|*[!0-9]*) n=60 ;; esac
  if [ "$INIT" = systemd ]; then
    journalctl -u xray -n "$n" --no-pager 2>/dev/null || tail -n "$n" "${LOG_DIR}/error.log"
  else
    tail -n "$n" "${LOG_DIR}/error.log" 2>/dev/null || tail -n "$n" "${LOG_DIR}/xray.err.log"
  fi
}

act_cron_restart() {
  need_root
  local hh
  askv hh "每天几点重启（0-23，留空关闭）" ""
  case "$hh" in
    '') : ;;
    *[!0-9]*) die "小时要是数字" ;;
    *) [ "$hh" -le 23 ] || die "小时要在 0-23" ;;
  esac
  if [ "$PKG" = apk ]; then
    mkdir -p /etc/crontabs
    if [ -f /etc/crontabs/root ]; then
      grep -v 'rs-auto-restart' /etc/crontabs/root > /etc/crontabs/root.tmp 2>/dev/null || : > /etc/crontabs/root.tmp
    else
      : > /etc/crontabs/root.tmp
    fi
    if [ -n "$hh" ]; then
      printf '0 %s * * * rc-service xray restart # rs-auto-restart\n' "$hh" >> /etc/crontabs/root.tmp
    fi
    mv /etc/crontabs/root.tmp /etc/crontabs/root
    chmod 600 /etc/crontabs/root
    rc-service crond restart >/dev/null 2>&1 || true
  else
    if [ -n "$hh" ]; then
      printf '0 %s * * * root systemctl restart xray # rs-auto-restart\n' "$hh" > /etc/cron.d/rs-xray
      chmod 644 /etc/cron.d/rs-xray
    else
      rm -f /etc/cron.d/rs-xray
    fi
  fi
  if [ -n "$hh" ]; then ok "已设置每天 ${hh} 点重启"; else ok "已关闭定时重启"; fi
}

# =============================================================================
# 配置 / 备份 / 内核
# =============================================================================
act_check_config() {
  need_root
  [ -f "$CONF_FILE" ] || die "没有 ${CONF_FILE}"
  "$BIN" run -test -c "$CONF_FILE" && ok "Xray 配置合法"
  nginx -t && ok "Nginx 配置合法"
}

act_backup() {
  need_root
  detect_os
  state_load || die "尚未安装"
  local out site
  out="/root/rs-backup-$(date +%Y%m%d-%H%M).tar.gz"
  site="${NGINX_SITE_DIR}/${DOMAIN}.conf"
  tar czf "$out" \
    usr/local/etc/xray usr/local/etc/rs \
    "etc/nginx/ssl/${DOMAIN}" \
    "${site#/}" \
    etc/systemd/system/xray.service etc/init.d/xray 2>/dev/null
  chmod 600 "$out"
  ok "备份：${out}"
}

act_restore() {
  need_root
  local f=${1:-}
  if [ -z "$f" ]; then
    printf '可用备份：\n'
    ls -1 /root/rs-backup-*.tar.gz 2>/dev/null || printf '  （无）\n'
    askv f "备份文件路径" ""
  fi
  [ -r "$f" ] || die "文件不可读：$f"
  tar xzf "$f" -C / || die "解包失败"
  ok "已恢复，正在重载"
  state_load || die "恢复后读不到状态"
  apply_all
}

act_upgrade_core() {
  need_root
  detect_os
  if [ -f "$LOCK_FILE" ]; then
    warn "内核已锁定 $(cat "$LOCK_FILE")，如需跟随最新版先在 [19] 解锁"
  fi
  state_load || die "尚未安装"
  rm -f "$BIN"
  step_bin
  "$BIN" run -test -c "$CONF_FILE" || die "新内核配置校验失败"
  svc_restart xray
  printf 'xray: %s\n' "$(svc_state xray)"
  "$BIN" version | head -1
}

act_core_version() {
  need_root
  local tag
  if [ -x "$BIN" ]; then
    printf '当前内核：%s\n' "$("$BIN" version 2>/dev/null | head -1)"
  else
    printf '当前内核：未安装\n'
  fi
  if [ -f "$LOCK_FILE" ]; then printf '锁定：%s\n' "$(cat "$LOCK_FILE")"; else printf '锁定：未锁定\n'; fi
  askv tag "锁定版本（如 v26.11.9；输入 unlock 解锁；回车不改）" ""
  case "$tag" in
    '')     ok "未改动" ;;
    unlock) rm -f "$LOCK_FILE"; ok "已解锁，可跟随最新版" ;;
    *)      printf '%s\n' "$tag" > "$LOCK_FILE"; chmod 600 "$LOCK_FILE"; ok "已锁定 ${tag}" ;;
  esac
}

act_self_update() {
  need_root
  local target
  target=$(command -v rs 2>/dev/null) || target=/usr/local/bin/rs
  log "从 ${RAW_BASE}/rs 拉取"
  curl -LfsS --max-time 60 "${RAW_BASE}/rs" -o "${target}.new" || die "下载失败"
  if ! bash -n "${target}.new"; then
    rm -f "${target}.new"
    die "新版本语法校验失败，已放弃"
  fi
  chmod +x "${target}.new"
  mv "${target}.new" "$target"
  ok "已更新 ${target}"
}

# =============================================================================
# 诊断 / 进阶
# =============================================================================
act_diag() {
  need_root
  state_load || die "尚未安装"
  SERVER_ADDR=${SERVER_ADDR:-$(public_ip)}
  local ip=$SERVER_ADDR d=$DOMAIN got
  printf '\n== 1. DNS ==\n'
  got=$(getent hosts "$d" 2>/dev/null | awk '{print $1}' | head -1) || got=""
  if [ "${got:-}" = "$ip" ]; then
    ok "解析一致：${got}"
  else
    warn "本地解析=${got:-无}，公网 IP=${ip}（Cloudflare 开橙云时会出现这种不一致）"
  fi

  printf '\n== 2. 端口归属 ==\n'
  ss -lntp 2>/dev/null | grep -E "(:443|:${TLS_PORT})" || warn "443 / ${TLS_PORT} 无监听"

  printf '\n== 3. 本机回落（Nginx 回环）==\n'
  openssl s_client -connect "127.0.0.1:${TLS_PORT}" -servername "$d" -alpn h2 -tls1_3 </dev/null 2>/dev/null \
    | grep -E 'Protocol|ALPN|subject=' || warn "本机回落握手失败"

  printf '\n== 4. 外部 443（应吐本机真证书）==\n'
  openssl s_client -connect "${ip}:443" -servername "$d" -alpn h2 -tls1_3 </dev/null 2>/dev/null \
    | grep -E 'Protocol|ALPN|subject=' || warn "外部 443 握手失败"

  printf '\n== 5. 证书 ==\n'
  if [ -s "/etc/nginx/ssl/${d}/fullchain.pem" ]; then
    openssl x509 -noout -subject -issuer -enddate -in "/etc/nginx/ssl/${d}/fullchain.pem" 2>/dev/null
    if openssl x509 -checkend 604800 -noout -in "/etc/nginx/ssl/${d}/fullchain.pem" >/dev/null 2>&1; then
      ok "有效期 >7 天"
    else
      warn "7 天内到期"
    fi
  else
    warn "没有证书文件"
  fi

  printf '\n== 6. 配置与服务 ==\n'
  if "$BIN" run -test -c "$CONF_FILE" >/dev/null 2>&1; then ok "xray 配置合法"; else warn "xray 配置有误"; fi
  printf 'xray=%s  nginx=%s  客户端=%s\n' "$(svc_state xray)" "$(svc_state nginx)" "$(clients_count)"
}

act_bbr() {
  need_root
  local cur yn
  cur=$(sysctl -n net.ipv4.tcp_congestion_control 2>/dev/null) || cur=unknown
  printf '当前拥塞控制：%s\n' "$cur"
  askv yn "启用 BBR？(y/N)" "N"
  if [ "$yn" != y ]; then ok "未改动"; return 0; fi
  modprobe tcp_bbr 2>/dev/null || true
  if ! grep -q '^net.core.default_qdisc' /etc/sysctl.conf 2>/dev/null; then
    printf 'net.core.default_qdisc=fq\nnet.ipv4.tcp_congestion_control=bbr\n' >> /etc/sysctl.conf
  fi
  sysctl -p >/dev/null 2>&1 || true
  printf '已设置：%s / %s\n' "$(sysctl -n net.core.default_qdisc 2>/dev/null)" "$(sysctl -n net.ipv4.tcp_congestion_control 2>/dev/null)"
}

act_firewall() {
  need_root
  local pick
  detect_os
  askv pick "1=按当前 SSH 端口重建规则  2=关闭防火墙" "1"
  case "$pick" in
    2)
      case "$PKG" in
        apk) rc-service nftables stop 2>/dev/null || true; rm -f /etc/nftables.nft ;;
        apt) ufw --force disable 2>/dev/null || true ;;
        dnf) systemctl stop firewalld 2>/dev/null || true ;;
      esac
      ok "防火墙已停" ;;
    *)
      askv SSH_PORT "SSH 端口" "22"
      fw_apply ;;
  esac
}

act_cert_only() {
  need_root
  state_load || die "尚未安装"
  askv CF_TOKEN "Cloudflare Token（留空走 HTTP-01）" ""
  if [ -n "$CF_TOKEN" ]; then ACME_MODE=dns_cf; else ACME_MODE=webroot; fi
  DO_CERT=1
  step_cert
  svc_reload nginx
}

act_uninstall() {
  need_root
  detect_os
  local yn deep="N"
  state_load >/dev/null 2>&1 || true
  askv yn "连站点、证书、状态与防火墙一起清？(y/N)" "N"
  [ "$yn" = y ] && deep=y
  svc_stop xray
  rm -f /etc/systemd/system/xray.service /etc/init.d/xray /etc/cron.d/rs-xray
  have systemctl && systemctl daemon-reload >/dev/null 2>&1
  rm -f "$BIN"
  rm -rf "$CONF_DIR" "$LOG_DIR"
  ok "节点已移除"
  if [ -n "${DOMAIN:-}" ]; then
    rm -f "${NGINX_SITE_DIR}/${DOMAIN}.conf" "/etc/nginx/sites-enabled/${DOMAIN}.conf"
    if nginx -t >/dev/null 2>&1; then svc_reload nginx; fi
    ok "站点 ${DOMAIN} 已移除"
  fi
  if [ "$deep" = y ]; then
    rm -rf "$STATE_DIR" "$LEGACY_INFO" "$ACME_WEBROOT"
    if [ -n "${DOMAIN:-}" ]; then
      rm -rf "/etc/nginx/ssl/${DOMAIN}" "/var/www/${DOMAIN}"
      if [ -x "${ACME_HOME}/acme.sh" ]; then
        "${ACME_HOME}/acme.sh" --remove -d "$DOMAIN" --ecc >/dev/null 2>&1 || true
      fi
    fi
    case "$PKG" in
      apk) rc-service nftables stop 2>/dev/null || true; rm -f /etc/nftables.nft ;;
      apt) ufw --force disable 2>/dev/null || true ;;
    esac
    ok "证书 / 站点 / 状态 / 防火墙已清理"
  fi
}

# =============================================================================
# 菜单
# =============================================================================
menu() {
  need_root
  detect_os
  state_init
  legacy_import
  local c=""
  while :; do
    printf '\n%s┌─ rs %s · REALITY 偷自己 ─────────────────%s\n' "$C_B" "$RS_VERSION" "$C_0"
    printf '│ 节点管理   1) 安装/重装节点   2) 添加客户端\n'
    printf '│            3) 链接与订阅     4) 删除客户端\n'
    printf '│            5) 修改节点       6) 轮换 REALITY 密钥\n'
    printf '│ 服务控制   7) 重启           8) 停止\n'
    printf '│            9) 启动          10) 运行状态\n'
    printf '│           11) 实时日志      12) 定时重启\n'
    printf '│ 配置更新  13) 检查配置      14) 备份\n'
    printf '│           15) 恢复备份      16) 更新脚本\n'
    printf '│           17) 重签证书\n'
    printf '│ 内核管理  18) 安装/更新内核 19) 锁定/切换版本\n'
    printf '│ 进阶      20) 诊断          21) BBR\n'
    printf '│           22) 防火墙        23) 卸载\n'
    printf '│  0) 退出\n'
    printf '%s└───────────────────────────────────────────%s\n' "$C_B" "$C_0"
    printf '选择: '
    read -r c || exit 0
    case "$c" in
      1)  act_install ;;
      2)  act_add_client ;;
      3)  export_links; act_list ;;
      4)  act_del_client ;;
      5)  act_edit ;;
      6)  act_rotate ;;
      7)  act_restart ;;
      8)  act_stop ;;
      9)  act_start ;;
      10) act_status ;;
      11) act_log ;;
      12) act_cron_restart ;;
      13) act_check_config ;;
      14) act_backup ;;
      15) act_restore ;;
      16) act_self_update ;;
      17) act_cert_only ;;
      18) act_upgrade_core ;;
      19) act_core_version ;;
      20) act_diag ;;
      21) act_bbr ;;
      22) act_firewall ;;
      23) act_uninstall ;;
      0|q|exit) exit 0 ;;
      *) warn "没有这个选项：$c" ;;
    esac
  done
}

usage() {
  cat <<'USAGE'
rs 2.0.0 —— REALITY「偷自己」节点管理器

用法：
  rs                                   交互菜单
  rs install --domain <fqdn> [选项]     安装节点
  rs add-client --email <备注>          追加客户端
  rs info                              查看链接
  rs export                            导出 links.txt / sub.txt / clash.yaml
  rs check                             诊断（DNS/端口/回落/证书/服务）
  rs backup | rs restore <file>        备份 / 恢复
  rs upgrade                           升级内核
  rs uninstall                         卸载（交互内可选连站点一起清）
  rs self-update                       更新本脚本

install 选项：
  --domain <fqdn>   --email <mail>   --network <raw|xhttp>
  --tls-port <port> --ssh-port <port> --cf-token <token>
  --xray-version <tag> --no-cert --no-firewall -y

以后直接运行：
  rs

单行安装：
  (curl -LfsS https://raw.githubusercontent.com/lrexionclub/reality-selfsteal/main/rs \
     -o /usr/local/bin/rs || wget -q https://raw.githubusercontent.com/lrexionclub/reality-selfsteal/main/rs \
     -O /usr/local/bin/rs) && chmod +x /usr/local/bin/rs && rs
USAGE
}

# ---------------------------------------------------------------- 入口
CLI_SUB=${1:-}
[ "$#" -gt 0 ] && shift

case "$CLI_SUB" in
  "")
    menu ;;
  install)
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --domain)       DOMAIN=${2:?}; shift 2 ;;
        --email)        EMAIL=${2:?}; shift 2 ;;
        --network)      NETWORK=${2:?}; shift 2 ;;
        --tls-port)     TLS_PORT=${2:?}; shift 2 ;;
        --ssh-port)     SSH_PORT=${2:?}; shift 2 ;;
        --cf-token)     CF_TOKEN=${2:?}; ACME_MODE=dns_cf; shift 2 ;;
        --xray-version) XRAY_VERSION=${2:?}; shift 2 ;;
        --no-cert)      DO_CERT=0; shift ;;
        --no-firewall)  DO_FIREWALL=0; shift ;;
        -y|--yes)       ASSUME_YES=1; shift ;;
        *) die "未知参数：$1" ;;
      esac
    done
    case "$NETWORK" in raw|xhttp) ;; *) die "--network 只能 raw 或 xhttp" ;; esac
    do_install ;;
  add-client)
    new_email=""
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --email) new_email=${2:?}; shift 2 ;;
        *) die "未知参数：$1" ;;
      esac
    done
    need_root
    state_load || die "尚未安装"
    [ -n "$new_email" ] || new_email="user$(( $(clients_count) + 1 ))@${DOMAIN}"
    new_uuid=$("$BIN" uuid)
    save_client "$new_uuid" "$new_email"
    state_save
    apply_all >/dev/null
    export_links >/dev/null
    link_for "$new_uuid" ;;
  info)        act_list ;;
  export)      export_links ;;
  check)       act_diag ;;
  backup)      act_backup ;;
  restore)     act_restore "${1:-}" ;;
  upgrade)     act_upgrade_core ;;
  uninstall)   act_uninstall ;;
  self-update) act_self_update ;;
  version|-v)  printf 'rs %s (REALITY self-steal)\n' "$RS_VERSION" ;;
  -h|--help|help) usage ;;
  *) die "未知子命令：${CLI_SUB}（rs --help）" ;;
esac
