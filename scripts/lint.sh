#!/usr/bin/env bash
# =============================================================================
# scripts/lint.sh —— 本地自检（等价于 .github/workflows/lint.yml 的全部检查项）
# 用法：bash scripts/lint.sh
#   依赖：bash、jq（可选）、shellcheck（可选）
# =============================================================================
set -uo pipefail

cd "$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)" || exit 1

FAIL=0
pass() { printf '\033[32m  ok  \033[0m %s\n' "$*"; }
bad()  { printf '\033[31m  FAIL\033[0m %s\n' "$*"; FAIL=$((FAIL + 1)); }
head_() { printf '\n\033[36m== %s\033[0m\n' "$*"; }

head_ "1. bash 语法（bash -n）"
for f in install.sh uninstall.sh lib/common.sh scripts/lint.sh rs; do
  if bash -n "$f" 2>/tmp/lint.err; then pass "$f"; else bad "$f"; sed 's/^/       /' /tmp/lint.err; fi
done

head_ "2. shellcheck"
if command -v shellcheck >/dev/null 2>&1; then
  for f in install.sh uninstall.sh lib/common.sh scripts/lint.sh rs; do
    if shellcheck -x -S warning -e SC1091,SC2317 "$f"; then pass "$f"; else bad "$f"; fi
  done
else
  printf '  skip 未安装 shellcheck（apk add shellcheck / apt install shellcheck）\n'
fi

head_ "3. JSON 模板（jq）"
if command -v jq >/dev/null 2>&1; then
  for f in conf/xray-reality-raw.json.tmpl conf/xray-reality-xhttp.json.tmpl; do
    sed -e 's|@UUID@|11111111-1111-1111-1111-111111111111|g' \
        -e 's|@DOMAIN@|example.com|g' \
        -e 's|@TLS_PORT@|8443|g' \
        -e 's|@PRIVKEY@|PRIVATEKEY|g' \
        -e 's|@SHORTID@|0123456789abcdef|g' \
        -e 's|@XPATH@|/abcdef123456|g' "$f" > /tmp/lint.json
    if jq -e . /tmp/lint.json >/dev/null; then pass "$f"; else bad "$f"; fi
  done
else
  printf '  skip 未安装 jq\n'
fi

head_ "4. 模板占位符清单"
for f in conf/*.tmpl; do
  printf '  %s: %s\n' "$f" "$(grep -oE '@[A-Z_]+@' "$f" | sort -u | tr '\n' ' ')"
done

head_ "5. rs CLI 冒烟"
for args in "version" "--help"; do
  if ./rs $args >/dev/null 2>&1; then pass "rs $args"; else bad "rs $args"; fi
done
out=$(./rs install --network bogus --domain a.b 2>&1 || true)
case "$out" in
  *"raw 或 xhttp"*) pass "rs install --network bogus 被正确拒绝" ;;
  *)                bad  "rs install --network bogus 未按预期拒绝：$out" ;;
esac
out=$(./rs install --domain 'bad domain!' 2>&1 || true)
case "$out" in
  *"非法字符"*) pass "非法域名被正确拒绝" ;;
  *)            bad  "非法域名未按预期拒绝：$out" ;;
esac

head_ "6. 行尾与可执行位"
for f in rs install.sh uninstall.sh lib/common.sh conf/openrc/xray; do
  if grep -q $'\r' "$f" 2>/dev/null; then bad "$f 含 CRLF"; else pass "$f LF"; fi
done
[ -x rs ] && pass "rs 可执行" || printf '  note rs 无 +x（chmod +x rs 即可）\n'

printf '\n'

head_ "7. 单文件模板渲染（rs 内嵌）"
marker=$(grep -n '入口' rs | tail -1 | cut -d: -f1)
if [ -n "${marker:-}" ]; then
  head -n $((marker - 1)) rs > /tmp/rs.lib
  tmpd=$(mktemp -d)
  (
    # shellcheck disable=SC1091
    . /tmp/rs.lib
    export DOMAIN=api.example.com TLS_PORT=8443 SITE_ROOT=/var/www/api.example.com
    export ACME_WEBROOT=/var/www/acme LOG_DIR=/var/log/xray BIN=/usr/local/bin/xray
    export CONF_FILE=/usr/local/etc/xray/config.json
    export UUID=11111111-1111-1111-1111-111111111111 PRIV=PRIVATEKEY
    export SHORTID=0123456789abcdef XPATH=/abcdef123456
    export NETWORK=raw;   tpl_xray > "$tmpd/raw.json"
    export NETWORK=xhttp; tpl_xray > "$tmpd/xhttp.json"
    tpl_nginx "" "    http2 on;"   > "$tmpd/site-new.conf"
    tpl_nginx " http2" "    # old" > "$tmpd/site-old.conf"
  )
  if grep -q '@[A-Z_]\+@' "$tmpd"/*.json "$tmpd"/*.conf 2>/dev/null; then
    bad "模板占位符未替换干净"
  else
    pass "占位符全部替换"
  fi
  grep -q 'listen 127.0.0.1:8443 ssl;'       "$tmpd/site-new.conf" && pass "nginx >=1.25.1 分支" || bad "nginx >=1.25.1 分支"
  grep -q 'listen 127.0.0.1:8443 ssl http2;' "$tmpd/site-old.conf" && pass "nginx <1.25.1 分支"  || bad "nginx <1.25.1 分支"
  grep -q '"dest": "127.0.0.1:8443"'         "$tmpd/raw.json"      && pass "xray raw dest"      || bad "xray raw dest"
  grep -q '"network": "xhttp"'               "$tmpd/xhttp.json"    && pass "xray xhttp network" || bad "xray xhttp network"
  if grep -q 'xtls-rprx-vision' "$tmpd/raw.json" && ! grep -q 'xtls-rprx-vision' "$tmpd/xhttp.json"; then
    pass "flow 只在 raw 出现"
  else
    bad "flow 与 network 不匹配"
  fi
  if command -v jq >/dev/null 2>&1; then
    jq -e . "$tmpd/raw.json"   >/dev/null && pass "raw.json 合法 JSON"   || bad "raw.json 非法"
    jq -e . "$tmpd/xhttp.json" >/dev/null && pass "xhttp.json 合法 JSON" || bad "xhttp.json 非法"
  fi
  rm -rf "$tmpd" /tmp/rs.lib
else
  bad "找不到 rs 的入口标记行"
fi
if [ "$FAIL" -eq 0 ]; then
  printf '\033[32m全部通过\033[0m\n'
  exit 0
fi
printf '\033[31m%d 项未通过\033[0m\n' "$FAIL"
exit 1
