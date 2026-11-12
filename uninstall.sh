#!/usr/bin/env bash
# =============================================================================
# reality-selfsteal · 卸载
#   bash uninstall.sh --domain api.example.com            # 只删节点，保留 Nginx 与站点
#   bash uninstall.sh --domain api.example.com --all      # 连站点、证书、防火墙一起清理
# =============================================================================
set -euo pipefail

if [ -z "${BASH_VERSION:-}" ]; then
  if command -v apk >/dev/null 2>&1; then
    apk add --no-cache bash >/dev/null 2>&1 || true
  fi
  exec bash "$0" "$@"
fi

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
# shellcheck source=lib/common.sh
. "${SCRIPT_DIR}/lib/common.sh"

DOMAIN=""
DO_ALL=0
DO_PURGE=0

while [ "$#" -gt 0 ]; do
  case "$1" in
    --domain) DOMAIN=${2:?}; shift 2 ;;
    --all)    DO_ALL=1; shift ;;
    --purge)  DO_ALL=1; DO_PURGE=1; shift ;;
    -h|--help) sed -n '2,7p' "$0"; exit 0 ;;
    *) die "未知参数：$1" ;;
  esac
done

[ "$(id -u)" -eq 0 ] || die "请用 root 运行"
detect_os

SSL_DIR=""
[ -n "$DOMAIN" ] && SSL_DIR=/etc/nginx/ssl/${DOMAIN}

log "卸载 Xray 服务"
svc_stop_disable xray
rm -f /etc/systemd/system/xray.service /etc/init.d/xray
have systemctl && systemctl daemon-reload >/dev/null 2>&1 || true
rm -f /usr/local/bin/xray
rm -rf /usr/local/etc/xray /var/log/xray
rm -f /root/reality-selfsteal.info
ok "Xray 已移除"

if [ -n "$DOMAIN" ]; then
  log "移除 Nginx 站点 ${DOMAIN}"
  rm -f "${NGINX_SITE_DIR}/${DOMAIN}.conf" "/etc/nginx/sites-enabled/${DOMAIN}.conf"
  if nginx -t >/dev/null 2>&1; then
    svc_reload nginx
    ok "Nginx 已重载"
  else
    warn "nginx -t 未通过，站点文件已删除，请手工检查"
  fi
fi

if [ "$DO_ALL" -eq 1 ] && [ -n "$DOMAIN" ]; then
  log "清理证书与站点目录"
  rm -rf "$SSL_DIR" "/var/www/${DOMAIN}" "/var/www/acme"
  if [ -x /root/.acme.sh/acme.sh ]; then
    /root/.acme.sh/acme.sh --remove -d "$DOMAIN" --ecc >/dev/null 2>&1 || true
    rm -rf "/root/.acme.sh/${DOMAIN}_ecc"
  fi
  ok "证书与站点目录已清理"
fi

if [ "$DO_ALL" -eq 1 ]; then
  log "回滚防火墙"
  fw_remove 22
  ok "防火墙规则已回滚"
fi

if [ "$DO_PURGE" -eq 1 ]; then
  log "卸载软件包"
  case "$PKG" in
    apk) pkg_remove nginx nftables ;;
    apt) pkg_remove nginx ufw ;;
    dnf) pkg_remove nginx firewalld ;;
  esac
  ok "软件包已卸载"
fi

printf '\n%s卸载完成。%s\n' "$C_GREEN" "$C_RESET"
