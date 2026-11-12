#!/usr/bin/env bash
# =============================================================================
# reality-selfsteal · 一键部署
#   VLESS + REALITY「偷自己」：dest 指向本机 Nginx，域名、证书、站点都在同一台机
#
#   支持：Alpine (apk / OpenRC) · Debian/Ubuntu (apt / systemd) · RHEL 系 (dnf / systemd)
#   依赖：nginx、openssl、curl、unzip、jq(可选)、acme.sh、Xray-core
#
#   用法：
#     bash install.sh --domain api.example.com --email you@mail.com
#     bash install.sh --domain api.example.com --email you@mail.com --network xhttp
#     bash install.sh --help
# =============================================================================
set -euo pipefail

# --- Alpine/busybox 默认没有 bash，先自举再重入 -------------------------------
if [ -z "${BASH_VERSION:-}" ]; then
  if command -v apk >/dev/null 2>&1; then
    apk add --no-cache bash >/dev/null 2>&1 || true
  fi
  exec bash "$0" "$@"
fi

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

# 一行流（bash <(curl ...) 或 curl | bash）拿不到仓库里的 conf/ 模板，
# 此时自动拉一份 tarball 到临时目录再重入。把下面的 slug 换成你自己的仓库。
REPO_SLUG="${XDD_REPO_SLUG:-liujihaodeku/reality-selfsteal}"
if [ ! -f "${SCRIPT_DIR}/conf/nginx-site.conf.tmpl" ]; then
  _tmp=$(mktemp -d)
  printf '[xdd] 本地无 conf/ 模板，尝试拉取 %s 源码包\n' "$REPO_SLUG" >&2
  if curl -fsSL --max-time 90 "https://codeload.github.com/${REPO_SLUG}/tar.gz/refs/heads/main" \
       | tar -xz -C "$_tmp" 2>/dev/null; then
    _dir=$(find "$_tmp" -maxdepth 1 -mindepth 1 -type d | head -1)
    if [ -n "${_dir:-}" ] && [ -f "${_dir}/install.sh" ]; then
      exec bash "${_dir}/install.sh" "$@"
    fi
  fi
  printf '[fail] 拉取模板失败。请 git clone 仓库后本地运行 install.sh，或设置 XDD_REPO_SLUG\n' >&2
  exit 1
fi

# shellcheck source=lib/common.sh
. "${SCRIPT_DIR}/lib/common.sh"

# ---------------------------------------------------------------- 默认值
DOMAIN=""
EMAIL=""
NETWORK="raw"                 # raw | xhttp
TLS_PORT=8443
SSH_PORT=22
CF_TOKEN=""
ACME_MODE="webroot"           # webroot | dns_cf
XRAY_VERSION=""
DO_FIREWALL=1
DO_CERT=1
ASSUME_YES=0
SITE_ROOT=""
XRAY_DIR=/usr/local/etc/xray
XRAY_CONF=${XRAY_DIR}/config.json
ACME_WEBROOT=/var/www/acme
ACME_HOME=/root/.acme.sh

usage() {
  cat <<'USAGE'
reality-selfsteal installer

  --domain <fqdn>        必填。已解析到本机的域名，例如 api.example.com
  --email <mail>         必填。acme.sh 注册邮箱
  --network <raw|xhttp>  传输层，默认 raw（VLESS + Vision）
  --tls-port <port>      Nginx 回环端口，默认 8443，必须与 dest 一致
  --ssh-port <port>      SSH 端口，默认 22（防火墙放行用）
  --cf-token <token>     Cloudflare API Token，填了就改用 DNS-01 签证书
  --xray-version <tag>   指定 Xray 版本，例如 v26.11.9，默认取最新
  --site-dir <path>      站点根目录，默认 /var/www/<domain>
  --no-firewall          跳过防火墙配置
  --no-cert              跳过证书申请（自行放置 fullchain.pem / privkey.pem）
  -y, --yes              无人值守，不询问
  -h, --help             显示本帮助
USAGE
}

# ---------------------------------------------------------------- 参数解析
while [ "$#" -gt 0 ]; do
  case "$1" in
    --domain)       DOMAIN=${2:?}; shift 2 ;;
    --email)        EMAIL=${2:?}; shift 2 ;;
    --network)      NETWORK=${2:?}; shift 2 ;;
    --tls-port)     TLS_PORT=${2:?}; shift 2 ;;
    --ssh-port)     SSH_PORT=${2:?}; shift 2 ;;
    --cf-token)     CF_TOKEN=${2:?}; ACME_MODE=dns_cf; shift 2 ;;
    --xray-version) XRAY_VERSION=${2:?}; shift 2 ;;
    --site-dir)     SITE_ROOT=${2:?}; shift 2 ;;
    --no-firewall)  DO_FIREWALL=0; shift ;;
    --no-cert)      DO_CERT=0; shift ;;
    -y|--yes)       ASSUME_YES=1; shift ;;
    -h|--help)      usage; exit 0 ;;
    *) die "未知参数：$1（--help 查看用法）" ;;
  esac
done

case "$NETWORK" in raw|xhttp) ;; *) die "--network 只能是 raw 或 xhttp" ;; esac

[ "$(id -u)" -eq 0 ] || die "请用 root 运行"
[ -n "$DOMAIN" ] || { [ -t 0 ] && read -r -p "域名（已解析到本机）: " DOMAIN || true; }
[ -n "$EMAIL" ]  || { [ -t 0 ] && read -r -p "邮箱（acme 注册用）: " EMAIL || true; }
[ -n "$DOMAIN" ] || die "缺少 --domain"
[ -n "$EMAIL" ]  || EMAIL="admin@${DOMAIN}"

case "$DOMAIN" in
  *[!a-zA-Z0-9.-]*) die "域名含非法字符：$DOMAIN" ;;
  *.*) ;;
  *) die "域名需要是完整 FQDN，例如 api.example.com" ;;
esac

SITE_ROOT=${SITE_ROOT:-/var/www/${DOMAIN}}
SSL_DIR=/etc/nginx/ssl/${DOMAIN}
INFO_FILE=/root/reality-selfsteal.info

# =============================================================================
# 步骤 1  环境
# =============================================================================
step_env() {
  log "步骤 1/7 环境与依赖"
  detect_os
  pkg_install nginx curl openssl unzip ca-certificates socat iproute2
  if [ "$PKG" = apk ]; then
    # Alpine 的 Xray 官方二进制为 glibc 静态构建；极端情况下用 gcompat 兜底
    pkg_install gcompat 2>/dev/null || true
    rc-update add crond default >/dev/null 2>&1 || true
    rc-service crond start  >/dev/null 2>&1 || true
  fi
  mkdir -p /var/log/xray "$XRAY_DIR" "$SITE_ROOT" "${ACME_WEBROOT}/.well-known/acme-challenge" "$SSL_DIR"
  ok "依赖就绪"
}

# =============================================================================
# 步骤 2  站点与 Nginx（把 443 让出来）
# =============================================================================
step_nginx() {
  log "步骤 2/7 站点与 Nginx"

  if [ -f "${SCRIPT_DIR}/conf/index.html" ]; then
    cp "${SCRIPT_DIR}/conf/index.html" "${SITE_ROOT}/index.html"
  fi

  # http2 指令语法在 1.25.1 变了，按版本生成
  local v
  v=$(nginx_version_num)
  if ver_ge "$v" 1.25.1; then
    LISTEN_HTTP2=""
    HTTP2_ON="    http2 on;"
  else
    LISTEN_HTTP2=" http2"
    HTTP2_ON="    # http2 已写在 listen 行（nginx < 1.25.1）"
  fi

  tpl "${SCRIPT_DIR}/conf/nginx-site.conf.tmpl" "${NGINX_SITE_DIR}/${DOMAIN}.conf"

  if [ "$PKG" != apk ]; then
    rm -f /etc/nginx/sites-enabled/default
    mkdir -p /etc/nginx/sites-enabled
    ln -sf "${NGINX_SITE_DIR}/${DOMAIN}.conf" "/etc/nginx/sites-enabled/${DOMAIN}.conf"
  else
    rm -f /etc/nginx/http.d/default.conf
  fi

  # 临时自签证书，让 nginx 能先起来跑 HTTP-01
  if [ ! -s "${SSL_DIR}/fullchain.pem" ]; then
    openssl req -x509 -nodes -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -days 2 \
      -keyout "${SSL_DIR}/privkey.pem" -out "${SSL_DIR}/fullchain.pem" \
      -subj "/CN=${DOMAIN}" >/dev/null 2>&1
  fi
  chown www-data:www-data "$SITE_ROOT" "$ACME_WEBROOT" 2>/dev/null || \
    chown "${NGINX_USER}:${NGINX_USER}" "$SITE_ROOT" "$ACME_WEBROOT" 2>/dev/null || true
  chmod 755 /var/www /var/www/acme 2>/dev/null || true

  nginx -t || die "nginx 配置校验失败"
  svc_enable_start nginx
  if [ "$PKG" = dnf ] && have restorecon; then
    restorecon -R /var/www /etc/nginx >/dev/null 2>&1 || true
  fi
  ok "Nginx 已启动，443 未被占用"
}

# =============================================================================
# 步骤 3  证书
# =============================================================================
step_cert() {
  if [ "$DO_CERT" -eq 0 ]; then
    warn "已按 --no-cert 跳过证书申请"
    return 0
  fi
  log "步骤 3/7 ACME 证书（${ACME_MODE}）"

  # 已有可用证书（剩余 ≥30 天且 SAN 匹配）就跳过，避免反复签发撞签发限额
  if [ -s "${SSL_DIR}/fullchain.pem" ] && \
     openssl x509 -checkend 2592000 -noout -in "${SSL_DIR}/fullchain.pem" >/dev/null 2>&1 && \
     openssl x509 -noout -text -in "${SSL_DIR}/fullchain.pem" 2>/dev/null | grep -q "$DOMAIN"; then
    ok "已有有效证书（剩余 >30 天），跳过签发"
    return 0
  fi

  if [ ! -x "${ACME_HOME}/acme.sh" ]; then
    curl -fsS https://get.acme.sh | sh -s email="$EMAIL" >/dev/null || die "acme.sh 安装失败"
  fi
  local acme="${ACME_HOME}/acme.sh"
  "$acme" --set-default-ca --server letsencrypt >/dev/null

  local resolved
  resolved=$(getent hosts "$DOMAIN" 2>/dev/null | awk '{print $1}' | head -1 || true)
  if [ -z "$resolved" ]; then
    warn "本地解析不到 ${DOMAIN}，继续尝试签发"
  fi

  if [ "$ACME_MODE" = dns_cf ]; then
    export CF_Token="$CF_TOKEN"
    "$acme" --issue --dns dns_cf -d "$DOMAIN" -k ec-256 --force || die "DNS-01 签发失败"
  else
    if ! "$acme" --issue -d "$DOMAIN" --webroot "$ACME_WEBROOT" --keylength ec-256 --force; then
      warn "HTTP-01 失败，改用 standalone 重试一次"
      svc_stop_disable nginx
      "$acme" --issue -d "$DOMAIN" --standalone --keylength ec-256 --force || die "证书签发失败"
      svc_enable_start nginx
    fi
  fi

  "$acme" --install-cert -d "$DOMAIN" --ecc \
    --key-file       "${SSL_DIR}/privkey.pem" \
    --fullchain-file "${SSL_DIR}/fullchain.pem" \
    --reloadcmd      "nginx -s reload" >/dev/null || die "证书安装失败"

  svc_reload nginx
  ok "证书已安装到 ${SSL_DIR}"
}

# =============================================================================
# 步骤 4  Xray-core 二进制
# =============================================================================
xray_arch() {
  case "$(uname -m)" in
    x86_64|amd64)   printf 'Xray-linux-64.zip' ;;
    aarch64|arm64)  printf 'Xray-linux-arm64-v8a.zip' ;;
    armv7l|armv7)   printf 'Xray-linux-arm32-v7a.zip' ;;
    i386|i686)      printf 'Xray-linux-32.zip' ;;
    s390x)          printf 'Xray-linux-s390x.zip' ;;
    *)              die "未适配的架构：$(uname -m)" ;;
  esac
}

step_xray_bin() {
  log "步骤 4/7 Xray-core"
  local asset tag url tmp
  asset=$(xray_arch)

  if [ -z "$XRAY_VERSION" ]; then
    tag=$(curl -fsS --max-time 10 https://api.github.com/repos/XTLS/Xray-core/releases/latest \
          | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -1) || true
    [ -n "${tag:-}" ] || tag="v26.11.9"
  else
    tag="$XRAY_VERSION"
  fi

  if [ -x /usr/local/bin/xray ] && /usr/local/bin/xray version 2>/dev/null | grep -q "${tag#v}"; then
    ok "已存在 Xray ${tag}，跳过下载"
    return 0
  fi

  url="https://github.com/XTLS/Xray-core/releases/download/${tag}/${asset}"
  tmp=$(mktemp -d)
  log "下载 ${tag} / ${asset}"
  curl -fL --retry 3 --max-time 300 -o "${tmp}/xray.zip" "$url" || die "Xray 下载失败：$url"
  unzip -o -q "${tmp}/xray.zip" -d "$tmp" || die "解压失败"
  install -m 0755 "${tmp}/xray" /usr/local/bin/xray
  rm -rf "$tmp"

  /usr/local/bin/xray version >/dev/null 2>&1 || die "Xray 二进制无法运行（架构/依赖不匹配）"
  ok "Xray ${tag} 安装到 /usr/local/bin/xray"
}

# =============================================================================
# 步骤 5  密钥与配置
# =============================================================================
step_config() {
  log "步骤 5/7 密钥与 Xray 配置"

  UUID=$(/usr/local/bin/xray uuid)
  local keyout
  keyout=$(/usr/local/bin/xray x25519)
  PRIV=$(printf '%s\n' "$keyout" | grep -i 'private' | awk -F': *' '{print $2}' | tr -d ' ' || true)
  PUB=$(printf  '%s\n' "$keyout" | grep -iE 'public|password' | awk -F': *' '{print $2}' | tr -d ' ' || true)
  if [ -z "$PRIV" ] && [ -n "$PUB" ]; then
    PRIV=$PUB; PUB=""
  fi
  [ -n "$PRIV" ] || die "x25519 解析失败，原始输出：$keyout"
  if [ -z "$PUB" ]; then
    PUB=$(/usr/local/bin/xray x25519 -i "$PRIV" | grep -iE 'public|password' | awk -F': *' '{print $2}' | tr -d ' ' || true)
  fi
  [ -n "$PUB" ] || die "公钥派生失败"
  SHORTID=$(rand_hex 8)
  XPATH="/$(rand_hex 6)"

  local tmpl
  if [ "$NETWORK" = xhttp ]; then
    tmpl="${SCRIPT_DIR}/conf/xray-reality-xhttp.json.tmpl"
  else
    tmpl="${SCRIPT_DIR}/conf/xray-reality-raw.json.tmpl"
  fi
  [ -f "$tmpl" ] || die "缺少模板：$tmpl"

  install -d -m 0755 "$XRAY_DIR"
  tpl "$tmpl" "$XRAY_CONF"
  # 640：root 可读写，服务账号靠属组读（属组在 step_service 里按 init 系统设定）
  chmod 640 "$XRAY_CONF"

  /usr/local/bin/xray run -test -c "$XRAY_CONF" || die "Xray 配置校验失败（看上面报错）"
  ok "配置写入 ${XRAY_CONF}"
}

# =============================================================================
# 步骤 6  服务
# =============================================================================
step_service() {
  log "步骤 6/7 服务注册"
  chmod 755 /var/log/xray
  case "$INIT" in
    systemd)
      local grp=nogroup
      getent group nogroup >/dev/null 2>&1 || grp=nobody
      # config 归 root:服务组，log 目录给服务组写权限，否则 nobody 打不开日志
      chgrp "$grp" "$XRAY_CONF" 2>/dev/null || true
      chown root:"$grp" /var/log/xray 2>/dev/null || true
      chmod 775 /var/log/xray
      sed "s|^Group=nogroup|Group=${grp}|" "${SCRIPT_DIR}/conf/systemd/xray.service" \
        > /etc/systemd/system/xray.service
      chmod 0644 /etc/systemd/system/xray.service
      systemctl daemon-reload
      systemctl enable xray >/dev/null 2>&1
      systemctl restart xray
      ;;
    openrc)
      install -m 0755 "${SCRIPT_DIR}/conf/openrc/xray" /etc/init.d/xray
      rc-update add xray default >/dev/null 2>&1 || true
      rc-service xray restart
      ;;
  esac
  sleep 2
  svc_state xray
  ss -lntp 2>/dev/null | grep -E '(:443|:'"$TLS_PORT"')' || warn "端口未就绪，看 /var/log/xray/error.log"
  ok "Xray 服务已启动"
}

# =============================================================================
# 步骤 7  防火墙 + 汇总
# =============================================================================
step_firewall() {
  if [ "$DO_FIREWALL" -eq 0 ]; then
    warn "已按 --no-firewall 跳过防火墙"
    return 0
  fi
  log "步骤 7/7 防火墙"
  fw_apply "$SSH_PORT"
}

write_info() {
  local ip proto link sni fp
  ip=$(public_ip)
  fp=chrome
  if [ "$NETWORK" = xhttp ]; then
    link="vless://${UUID}@${ip}:443?encryption=none&security=reality&sni=${DOMAIN}&fp=${fp}&pbk=${PUB}&sid=${SHORTID}&type=xhttp&path=${XPATH}&mode=auto#${DOMAIN}-xhttp"
    proto="XHTTP + REALITY"
  else
    link="vless://${UUID}@${ip}:443?encryption=none&flow=xtls-rprx-vision&security=reality&sni=${DOMAIN}&fp=${fp}&pbk=${PUB}&sid=${SHORTID}&type=tcp#${DOMAIN}"
    proto="RAW/TCP + REALITY + Vision"
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

  printf '\n%s==================== 部署完成 ====================%s\n' "$C_GREEN" "$C_RESET"
  printf '域名        : %s\n' "$DOMAIN"
  printf '协议        : %s\n' "$proto"
  printf '服务器 IP    : %s\n' "$ip"
  printf 'UUID        : %s\n' "$UUID"
  printf '公钥 pbk     : %s\n' "$PUB"
  printf 'shortId     : %s\n' "$SHORTID"
  [ "$NETWORK" = xhttp ] && printf 'XHTTP path  : %s\n' "$XPATH"
  printf 'dest        : 127.0.0.1:%s (Nginx)\n' "$TLS_PORT"
  printf '监听        : 0.0.0.0:443 (Xray)\n'
  printf '\n客户端链接：\n%s\n' "$link"
  printf '\n凭据文件    : %s (600)\n' "$INFO_FILE"
  printf '%s==================================================%s\n\n' "$C_GREEN" "$C_RESET"

  printf '自检命令：\n'
  printf '  ss -lntp | grep -E ":443|:%s"\n' "$TLS_PORT"
  printf '  curl -sI https://%s/ | head -3\n' "$DOMAIN"
  printf '  openssl s_client -connect %s:443 -servername %s -alpn h2 -tls1_3 </dev/null 2>/dev/null | grep -E "Protocol|ALPN|subject="\n' "$ip" "$DOMAIN"
  printf '  journalctl -u xray -n 30 --no-pager    # Alpine 用: cat /var/log/xray/error.log\n\n'
}

main() {
  [ "$ASSUME_YES" -eq 1 ] && log "无人值守模式（-y）"
  step_env
  step_nginx
  step_cert
  step_xray_bin
  step_config
  step_service
  step_firewall
  write_info
}

main "$@"
