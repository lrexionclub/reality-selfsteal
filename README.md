# reality-selfsteal

> 纯手写 **VLESS + REALITY「偷自己」**：`dest` 指向本机 Nginx，域名、证书、站点全在同一台机器上。
> 不要宝塔，不要 3x-ui，不要 aaPanel / 1Panel。三条命令装完。

[![lint](https://github.com/lrexionclub/reality-selfsteal/actions/workflows/lint.yml/badge.svg)](https://github.com/lrexionclub/reality-selfsteal/actions/workflows/lint.yml)
[![license](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![platform](https://img.shields.io/badge/platform-Alpine%20%7C%20Debian%20%7C%20Ubuntu%20%7C%20RHEL-informational)](#支持矩阵)

---

## 为什么是「偷自己」

REALITY 本身不需要域名，它借谁的脸，外面看到的就是谁。以前大家把 `dest` 随手填成微软、苹果，
于是出现了一个硬伤：

| 探测者看到 | 结论 |
|---|---|
| TLS 证书写着 `www.microsoft.com` | 正常 |
| 这个证书却挂在搬瓦工洛杉矶某台 IP 上 | 地址与身份对不上，特征成立 |
| 每次探测，服务器都要跑去连一次远方的微软再转回来 | 握手延迟异常，特征成立 |

「偷自己」把这张脸换成自己的：域名是你注册的，DNS 指向这台机，证书在这台机签的，网站真跑在这台机。
怎么查都对得上。

```
                 ┌──────────────── 你的服务器 ────────────────┐
  客户端 ──443──▶│ Xray (REALITY)                              │
                 │   ├─ 暗号对上 ──▶ VLESS 出站 ──▶ 互联网      │
                 │   └─ 暗号不对 ──▶ 原样转发 ──▶ Nginx:8443   │
                 │                                  │           │
  探测者 ──443──▶│ ─────────────── 同一条回落链路 ───┘           │
                 │                                  ▼           │
                 │                     真域名 + 真证书 + 真站点  │
                 └─────────────────────────────────────────────┘
```

## 特性

- **零面板**：不引入任何 Web 控制台，攻击面只有 22 / 80 / 443
- **跨发行版**：Alpine（`apk` + OpenRC）、Debian / Ubuntu（`apt` + systemd）、RHEL 系（`dnf` + systemd）
- **443 只给 Xray**：Nginx 退到 `127.0.0.1:8443`，外部扫这个端口什么都扫不到
- **两种传输**：`raw`（VLESS + Vision，默认）与 `xhttp`（XHTTP + REALITY），一条参数切换
- **证书自动化**：acme.sh，支持 HTTP-01（webroot）与 Cloudflare DNS-01（`--cf-token`）
- **防火墙自适应**：Alpine 用 nftables，Debian/Ubuntu 用 ufw，RHEL 用 firewalld
- **幂等**：重复执行只覆盖配置，不重复下载二进制
- **可卸载**：`uninstall.sh` 分级清理，不动无关服务

## 支持矩阵

| 系统 | 包管理 | init | 防火墙 | 状态 |
|---|---|---|---|---|
| Alpine 3.18 / 3.19 / 3.20 / 3.21 | apk | OpenRC | nftables | 支持 |
| Debian 11 / 12 / 13 | apt | systemd | ufw | 支持 |
| Ubuntu 20.04 / 22.04 / 24.04 | apt | systemd | ufw | 支持 |
| Rocky / AlmaLinux / RHEL 8+ | dnf | systemd | firewalld | 支持 |

架构：`x86_64`、`aarch64`、`armv7l`、`i386`、`s390x`（与 Xray 官方 release 资产一一对应）。

## 需要准备什么（以及为什么）

`rs` 装节点前会挨个问五个值，`install.sh` 用同名参数。前三个决定节点通不通，后两个决定你会不会被自己关在门外。

| # | 项目 | 必填 | 默认 | 参数 | 落到哪里 |
|---|---|---|---|---|---|
| 1 | 域名 | 是 | 上次用过的 | `--domain` | Nginx `server_name`、证书 SAN、REALITY `serverNames`、客户端 `sni` |
| 2 | 邮箱 | 否 | `admin@<域名>` | `--email` | acme.sh 的 ACME 账户 |
| 3 | 回环端口 | 否 | `8443` | `--tls-port` | Nginx `listen 127.0.0.1:<port>` 与 Xray `dest`（两处成对） |
| 4 | SSH 端口 | 否 | `22` | `--ssh-port` | 防火墙里唯一一条 SSH 放行规则 |
| 5 | CF Token | 否 | 空（走 HTTP-01） | `--cf-token` | `acme.sh --dns dns_cf` 签发 |

### 1. 域名（必填）

要一条**已存在、指向这台机器公网 IPv4** 的 A 记录，配合一个完整 FQDN。

```sh
curl -4 -s https://api.ipify.org; echo    # 本机公网 IP
dig +short api.example.com                # 必须与上一行一致
curl -sI http://api.example.com/ | head -1 # 走 HTTP-01 时 80 要能到
```

| 合格 | `api.example.com`、`node.mydomain.net` |
|---|---|
| 不合格 | `1.2.3.4`（IP）、`http://api.example.com`（带协议）、`api.example.com:443`（带端口）、`api.example.com/`（带路径） |

脚本把它写进四处：

```nginx
server_name api.example.com;
```

```json
"serverNames": [ "api.example.com" ],
"dest": "127.0.0.1:8443"
```

```
vless://<UUID>@<IP>:443?...#api.example.com   中的 sni=api.example.com
```

**为什么必须是自己的真域名**：REALITY 的伪装原则是「外面看到的脸 = dest 的脸」。探测者拿这个 SNI 连 443，Xray 判定不是自己人，就把连接原样甩给 Nginx，他收到的证书写着 `api.example.com`，DNS 查这个域名正好指向这台 IP，证书也确实是这台机签的，三样对齐。换成借大站证书挂在机房 IP 上的搭法，证书写着微软却指向搬瓦工，地址与身份错位，本身就是特征。

Cloudflare 那一侧只做解析，**云朵必须灰（DNS only）**：橙云会让域名解析出 CF 的 IP，而客户端连的是你的真实 IP，对齐关系当场断掉。

### 2. 邮箱（可省）

一个能收信的地址即可，不必与域名同域。脚本只做一件事：

```sh
curl -fsS https://get.acme.sh | sh -s email="$EMAIL"
```

acme.sh 拿它向 Let's Encrypt 注册 ACME 账户，写进 `/root/.acme.sh/account.conf`。它不参与签名、不进节点配置。证书即将到期且自动续签连续失败、或证书被吊销时，CA 会往这个地址发通知。默认值 `admin@<域名>` 只是让这一栏不为空值。

### 3. 回环端口（默认 8443）

Nginx 从这个端口接收 Xray 回落过来的流量，所以它要**只绑回环**、且**不与 443 冲突**。

```sh
ss -lntp | grep 8443        # 有输出就换一个，例如 9443
```

两处必须一致：

```nginx
listen 127.0.0.1:8443 ssl;   # 127.0.0.1 前缀 = 外部扫不到
```

```json
"dest": "127.0.0.1:8443"
```

**为什么要挪到回环**：443 得让给 Xray（真实 HTTPS 站点都在 443，Reality 站在别的端口上本身就异常）；绑 `0.0.0.0:8443` 的话，别人扫端口会在 8443 上发现一个和你域名一模一样的站点，等于在地上留了第二串脚印。

### 4. SSH 端口（默认 22）

你当前 sshd 正在监听的端口。脚本只把它写进防火墙放行，**不动 `sshd_config`、不重启 sshd**。

```sh
ss -lntp | grep sshd        # 实际监听端口
```

```sh
# Alpine / nftables
tcp dport { 22, 80, 443 } accept
# Debian / Ubuntu / ufw
ufw allow 22/tcp
```

**为什么必须问**：最后一步是默认拒绝入站（nftables `policy drop` / `ufw default deny incoming`）。写死 22 而你的 sshd 在 2222，规则一生效，当前会话立刻冻住，重连被拒，只能从云厂商控制台进 VNC 救。云厂商安全组是另一层，那边也要放行同一个端口。

### 5. CF Token（可省，留空走 HTTP-01）

Cloudflare **API Token**，不是 Global API Key。创建路径：Dashboard → My Profile → API Tokens → Create Token → 模板 *Edit zone DNS* → 权限 `Zone:Read` + `DNS:Edit` → Zone Resources 指定那一个域名。

脚本用它跑 DNS-01：

```sh
export CF_Token="$CF_TOKEN"
acme.sh --issue --dns dns_cf -d "$DOMAIN" -k ec-256 --force
```

acme.sh 往你的 zone 写一条 `_acme-challenge.<域名>` TXT 完成校验，签完删掉；Token 被存进 `account.conf`（600）供三个月后自动续签使用。

| 场景 | 留空（HTTP-01） | 填 Token（DNS-01） |
|---|---|---|
| 80 空着、域名已解析 | 能签 | 能签 |
| 80 被占用 / 云厂商封 80 | 签不下来 | 能签 |
| 域名刚买、解析未生效 | 失败 | 能签 |
| 泛域名 `*.example.com` | 不行 | 能签 |

### 五个值填完长这样

```
域名      api.example.com
邮箱      me@gmail.com
回环端口  8443
SSH 端口  2222
CF Token  （留空，回车）
```

落盘结果：

```
/etc/nginx/http.d/api.example.com.conf        Alpine
/etc/nginx/sites-available/api.example.com.conf  Debian / Ubuntu
/etc/nginx/ssl/api.example.com/{fullchain,privkey}.pem
/usr/local/etc/xray/config.json
/root/.acme.sh/account.conf
/root/reality-selfsteal.info
```

## 快速开始

### Alpine

```sh
apk add --no-cache bash git curl
git clone https://github.com/lrexionclub/reality-selfsteal.git
cd reality-selfsteal
bash install.sh --domain api.example.com --email you@mail.com
```

### Debian / Ubuntu

```sh
apt update && apt install -y git curl
git clone https://github.com/lrexionclub/reality-selfsteal.git
cd reality-selfsteal
bash install.sh --domain api.example.com --email you@mail.com
```

### 单文件一键（推荐：`rs`）

```sh
(curl -LfsS https://raw.githubusercontent.com/lrexionclub/reality-selfsteal/main/rs \
   -o /usr/local/bin/rs || wget -q https://raw.githubusercontent.com/lrexionclub/reality-selfsteal/main/rs \
   -O /usr/local/bin/rs) && chmod +x /usr/local/bin/rs && rs
```

`rs` 是自包含单文件：五套模板（Nginx vhost、Xray raw、Xray xhttp、systemd unit、OpenRC 脚本）
全部内嵌在脚本里，不依赖 `conf/` 目录。装完直接再敲 `rs` 就是菜单。

| 子命令 | 作用 |
|---|---|
| `rs` | 交互菜单（安装 / 信息 / 自检 / 升级 / 卸载 / 自更新） |
| `rs install --domain a.example.com --email you@mail.com` | 直接安装，RAW + Vision |
| `rs install --domain a.example.com --email you@mail.com --network xhttp` | 直接安装，XHTTP |
| `rs info` | 打印凭据与客户端链接 |
| `rs check` | 端口归属 / 回落链路 / 证书 / 服务 六项自检 |
| `rs upgrade` | 升级 Xray 内核（先 `run -test` 再重启） |
| `rs uninstall [--all]` | 卸载，`--all` 连站点/证书/防火墙 |
| `rs self-update` | 重新下载本脚本（下载后先 `bash -n` 再覆盖） |

### 一行流（多文件版 install.sh）

```sh
bash <(curl -fsSL https://raw.githubusercontent.com/lrexionclub/reality-selfsteal/main/install.sh) \
  --domain api.example.com --email you@mail.com
```

> 多文件版会先把自身需要的 `conf/` 模板从仓库 tarball 拉到临时目录再重入执行。
> 仓库 slug 写死在 `install.sh` 顶部的 `REPO_SLUG`，换成你自己的即可，
> 也可以用 `XDD_REPO_SLUG=you/reality-selfsteal bash install.sh ...` 临时覆盖。

### 变体

```sh
# Cloudflare DNS-01 签证书（不占 80 端口）
bash install.sh --domain api.example.com --email you@mail.com --cf-token $CF_TOKEN

# XHTTP + REALITY
bash install.sh --domain api.example.com --email you@mail.com --network xhttp

# 改 Nginx 回环端口 / SSH 端口
bash install.sh --domain api.example.com --email you@mail.com --tls-port 9443 --ssh-port 2222

# 自己已有证书，跳过申请
bash install.sh --domain api.example.com --email you@mail.com --no-cert

# 不动防火墙
bash install.sh --domain api.example.com --email you@mail.com --no-firewall
```

## 参数

| 参数 | 默认 | 说明 |
|---|---|---|
| `--domain` | 无（必填） | 已解析到本机的 FQDN |
| `--email` | `admin@<domain>` | acme.sh 注册邮箱 |
| `--network` | `raw` | `raw`（VLESS+Vision）或 `xhttp` |
| `--tls-port` | `8443` | Nginx 回环监听端口，必须与 `dest` 一致 |
| `--ssh-port` | `22` | 防火墙放行用，别把自己关在门外 |
| `--cf-token` | 空 | 填了自动切 DNS-01（`dns_cf`） |
| `--xray-version` | 最新 | 例如 `v26.11.9` |
| `--site-dir` | `/var/www/<domain>` | 站点根目录 |
| `--no-firewall` | 关 | 跳过防火墙配置 |
| `--no-cert` | 关 | 跳过证书申请 |
| `-y`, `--yes` | 关 | 无人值守 |

## 装完自检

```sh
# 1. 端口归属：443 是 xray，8443 只有回环
ss -lntp | grep -E ':443|:8443'
#   期望：LISTEN 0 ... 0.0.0.0:443   users:(("xray",...))
#         LISTEN 0 ... 127.0.0.1:8443 users:(("nginx",...))

# 2. 站点正常
curl -sI https://api.example.com/ | head -3

# 3. 核心验证：无暗号连接被原样转给 Nginx，吐你本机的真证书
openssl s_client -connect <服务器IP>:443 -servername api.example.com -alpn h2 -tls1_3 </dev/null 2>/dev/null \
  | grep -E 'Protocol|ALPN|subject='
#   期望：Protocol : TLSv1.3 / ALPN protocol: h2 / subject=CN=api.example.com

# 4. 服务状态
systemctl status xray --no-pager        # Debian / Ubuntu / RHEL
rc-service xray status                  # Alpine
tail -n 30 /var/log/xray/error.log
```

凭据落在 `/root/reality-selfsteal.info`（权限 600）：`uuid` / `public_key` / `private_key` / `short_id` / `url`。

## 客户端链接

`raw` + Vision：

```
vless://<UUID>@<服务器IP>:443?encryption=none&flow=xtls-rprx-vision&security=reality&sni=api.example.com&fp=chrome&pbk=<公钥>&sid=<shortId>&type=tcp#api.example.com
```

`xhttp`：

```
vless://<UUID>@<服务器IP>:443?encryption=none&security=reality&sni=api.example.com&fp=chrome&pbk=<公钥>&sid=<shortId>&type=xhttp&path=<XPATH>&mode=auto#api.example.com-xhttp
```

| 客户端 | 注意点 |
|---|---|
| v2rayN / v2rayNG | 内核升到 Xray ≥ 1.8.24；`sni` 必须等于域名 |
| NekoBox / sing-box | 选 VLESS + REALITY，`pbk` 填公钥，`sid` 填 shortId |
| Shadowrocket | 手动添加，类型 VLESS，传输 `tcp`（或 `xhttp`） |
| Clash Meta / mihomo | 内核要支持 `reality-opts` |

## 仓库结构

```
reality-selfsteal/
├── rs                               # 单文件自包含安装器（推荐，模板全内嵌）
├── install.sh                       # 主安装脚本（7 步，读 conf/ 模板）
├── uninstall.sh                     # 卸载（--all / --purge）
├── lib/
│   └── common.sh                    # 发行版探测 / 包管理 / 服务 / 防火墙 / 模板渲染
├── conf/
│   ├── nginx-site.conf.tmpl         # Nginx vhost：80 跳转 + 127.0.0.1:8443
│   ├── index.html                   # 占位首页
│   ├── xray-reality-raw.json.tmpl   # VLESS + REALITY + Vision
│   ├── xray-reality-xhttp.json.tmpl # VLESS + REALITY + XHTTP
│   ├── systemd/xray.service
│   └── openrc/xray                  # Alpine 用的 OpenRC 脚本
├── .github/workflows/lint.yml       # bash -n + shellcheck + jq 校验模板 + rs 冒烟
├── LICENSE
└── README.md
```

## 多用户与流量统计

面板没了，这两件事手动做：

**加用户**——在 `config.json` 的 `clients` 数组里追加一项，`xray run -test` 通过后重启：

```json
{ "id": "第二个UUID", "flow": "xtls-rprx-vision", "email": "user2@api.example.com" }
```

`xray uuid` 生成 UUID。`xhttp` 模式去掉 `flow` 字段。

**流量统计**——打开 Xray 自带 Stats API：

```json
"api": { "tag": "api", "services": [ "StatsService" ] },
"stats": {},
"policy": { "levels": { "0": { "statsUserUplink": true, "statsUserDownlink": true } } }
```

再加一个只监听回环的 inbound 和一条路由：

```json
{ "listen": "127.0.0.1", "port": 10085, "protocol": "dokodemo-door", "tag": "api",
  "settings": { "address": "127.0.0.1" } }
```

```json
{ "type": "field", "inboundTag": [ "api" ], "outboundTag": "api" }
```

查询：

```sh
xray api statsquery --server=127.0.0.1:10085 -pattern ""
xray api statsonline --server=127.0.0.1:10085
```

## 常见问题

| 现象 | 原因 | 处理 |
|---|---|---|
| 启动报 `empty "serverNames"` | `serverNames` 空 | 填真域名，且与证书 SAN 一致 |
| `address already in use` | 有 vhost 还占着 443 | `grep -rn "listen.*443" /etc/nginx/`，改成 `127.0.0.1:8443`，`[::]:443` 一起删 |
| 浏览器「重定向次数过多」 | 跳转条件写成 `!= 443` | 只对 80 跳转 |
| 探测报 `certificate not trusted` | 用 `127.0.0.1` 去对域名证书 | 正常现象，只看 `TLSv1.3` 与 `h2` |
| 握手失败，日志 `REALITY: processed invalid connection` | `pbk` 与 `privateKey` 不是一对 | `xray x25519 -i <私钥>` 重算公钥 |
| Alpine 上 nginx 起不来 | 还留着 `http.d/default.conf` | `rm /etc/nginx/http.d/default.conf`，脚本已处理 |
| Alpine 上 xray 报错无法执行 | 缺少兼容层 | `apk add gcompat`，脚本已尝试安装 |
| 证书 60 天后失效 | `--reloadcmd` 丢了 | 重跑 `acme.sh --install-cert -d <domain> --ecc ...` |
| Cloudflare 开了代理 | 解析出来是 CF 的 IP | 灰云（DNS only） |

## 卸载

```sh
# 只删节点，Nginx 与站点保留
bash uninstall.sh --domain api.example.com

# 节点 + 站点 + 证书 + 防火墙
bash uninstall.sh --domain api.example.com --all

# 连 nginx / ufw / nftables 一起卸
bash uninstall.sh --domain api.example.com --purge
```

## 声明

```
本项目仅用于计算机网络协议、TLS 握手与流量伪装技术的学习与研究，仅供学习使用。
请在您拥有合法控制权的服务器与域名上使用；使用者需自行遵守所在地适用的法律法规。
作者不对任何使用后果承担责任，也不提供任何形式的担保。
```

## 许可

[MIT](LICENSE)

## 致谢

- [XTLS/Xray-core](https://github.com/XTLS/Xray-core) —— VLESS / REALITY / XHTTP / Vision
- [XTLS/REALITY](https://github.com/XTLS/REALITY) —— REALITY 协议与设计说明
- [acmesh-official/acme.sh](https://github.com/acmesh-official/acme.sh) —— 证书自动化
- [MHSanaei/3x-ui](https://github.com/MHSanaei/3x-ui) —— 面板方案的参照实现

## 参考

- REALITY 官方说明：<https://github.com/XTLS/REALITY>
- XHTTP 五合一配置讨论：<https://github.com/XTLS/Xray-core/discussions/4118>
- Xray 官方安装脚本：<https://github.com/XTLS/Xray-install>
