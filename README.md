# reality-selfsteal

> 纯手写 **VLESS + REALITY「偷自己」**：`dest` 指向本机 Nginx，域名、证书、站点全在同一台机器上。
> 不要宝塔，不要 3x-ui，不要 aaPanel / 1Panel。三条命令装完。

[![lint](https://github.com/lrexion/reality-selfsteal/actions/workflows/lint.yml/badge.svg)](https://github.com/lrexion/reality-selfsteal/actions/workflows/lint.yml)
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

## 主要能力

- **零面板**：不引入 Web 控制台，对外只有 22 / 80 / 443。
- **单文件 `rs`**：23 项菜单，安装、多客户端、订阅导出、备份、诊断、内核锁定、卸载都在一个文件里。
- **多客户端**：一个入站挂多个 UUID，增删即时重载，链接与订阅自动刷新。
- **订阅导出**：`links.txt`（明文）、`sub.txt`（base64 订阅）、`clash.yaml`（Mihomo `reality-opts`）。
- **两种传输**：`raw`（VLESS + Vision，默认）与 `xhttp`（XHTTP + REALITY），切换时自动同步各客户端 `flow`。
- **三种 init**：Alpine `apk` + OpenRC、Debian/Ubuntu `apt` + systemd、RHEL 系 `dnf` + systemd。
- **防火墙自适应**：Alpine 写 nftables 规则集，Debian/Ubuntu 走 ufw，RHEL 走 firewalld。
- **证书自动化**：acme.sh，HTTP-01（webroot）或 Cloudflare DNS-01（`--cf-token`），到期前自动跳过重复签发。
- **内核治理**：最新版 / 指定版本 / 版本锁定（`version.lock`），升级前先 `run -test`，失败不重启。
- **备份恢复**：一条命令打包配置、状态、证书与 service 文件，恢复后自动重载。
- **诊断**：DNS 对齐、端口归属、本机回落、外部 443 真证书、证书余期、配置与服务状态，一次全查。
- **幂等**：重复执行只覆盖配置，不重复下载二进制。

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
git clone https://github.com/lrexion/reality-selfsteal.git
cd reality-selfsteal
bash install.sh --domain api.example.com --email you@mail.com
```

### Debian / Ubuntu

```sh
apt update && apt install -y git curl
git clone https://github.com/lrexion/reality-selfsteal.git
cd reality-selfsteal
bash install.sh --domain api.example.com --email you@mail.com
```

### 单文件一键（推荐：`rs`）

```sh
(curl -LfsS https://raw.githubusercontent.com/lrexion/reality-selfsteal/main/rs \
   -o /usr/local/bin/rs || wget -q https://raw.githubusercontent.com/lrexion/reality-selfsteal/main/rs \
   -O /usr/local/bin/rs) && chmod +x /usr/local/bin/rs && rs
```

`rs` 是自包含单文件：Nginx vhost、Xray（raw / xhttp 两套）、systemd unit、OpenRC 脚本
全部内嵌，不依赖 `conf/` 目录。

以后直接运行：

```sh
rs
```

首次进入会自检依赖与状态目录，之后每次打开都是同一个菜单。

| 子命令 | 作用 |
|---|---|
| `rs` | 交互菜单（23 项） |
| `rs install --domain a.example.com --email you@mail.com` | 直接安装，RAW + Vision |
| `rs install --domain a.example.com --email you@mail.com --network xhttp` | 直接安装，XHTTP |
| `rs add-client --email user2` | 追加客户端并打印链接 |
| `rs info` | 节点信息 + 全部客户端链接 |
| `rs export` | 导出 `links.txt` / `sub.txt` / `clash.yaml` |
| `rs check` | 六段诊断：DNS / 端口 / 回落 / 外部 443 / 证书 / 服务 |
| `rs backup` / `rs restore <file>` | 备份与恢复 |
| `rs upgrade` | 升级 Xray 内核（先 `run -test` 再重启） |
| `rs uninstall` | 卸载，交互里可选连站点/证书/防火墙一起清 |
| `rs self-update` | 重新下载本脚本（下载后先 `bash -n` 再覆盖） |

状态与导出目录：`/usr/local/etc/rs/`（`state.conf` / `clients.conf` / `version.lock` / `export/`），权限 600。

### 一行流（多文件版 install.sh）

```sh
bash <(curl -fsSL https://raw.githubusercontent.com/lrexion/reality-selfsteal/main/install.sh) \
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

## 主菜单

```
┌─ rs 2.0.0 · REALITY 偷自己 ─────────────────
│ 节点管理   1) 安装/重装节点   2) 添加客户端
│            3) 链接与订阅     4) 删除客户端
│            5) 修改节点       6) 轮换 REALITY 密钥
│ 服务控制   7) 重启           8) 停止
│            9) 启动          10) 运行状态
│           11) 实时日志      12) 定时重启
│ 配置更新  13) 检查配置      14) 备份
│           15) 恢复备份      16) 更新脚本
│           17) 重签证书
│ 内核管理  18) 安装/更新内核 19) 锁定/切换版本
│ 进阶      20) 诊断          21) BBR
│           22) 防火墙        23) 卸载
└───────────────────────────────────────────
```

| 分类 | 编号 | 功能 | 落到哪里 |
|---|---:|---|---|
| 节点管理 | 1 | 安装 / 重装节点（raw 或 xhttp） | `/usr/local/etc/xray/config.json`、Nginx vhost |
| | 2 | 添加客户端（多 UUID） | `state.conf` + `clients.conf` + 配置重载 |
| | 3 | 链接与订阅 | `/usr/local/etc/rs/export/` |
| | 4 | 删除客户端 | 配置重载，至少保留一个 |
| | 5 | 修改节点（域名 / 网络 / 回环端口 / 连接地址） | 改域名会触发重签证书 |
| | 6 | 轮换 REALITY 密钥 | 旧链接全部作废，pubkey 立即更新 |
| 服务控制 | 7-9 | 重启 / 停止 / 启动 | `xray` service |
| | 10 | 运行状态（含端口、内核版本、证书余期） | — |
| | 11 | 实时日志 | journald 或 `/var/log/xray/` |
| | 12 | 定时重启 | `/etc/cron.d/rs-xray` 或 `/etc/crontabs/root` |
| 配置更新 | 13 | 检查配置 | `xray run -test` + `nginx -t` |
| | 14-15 | 备份 / 恢复 | `/root/rs-backup-<时间>.tar.gz` |
| | 16 | 更新脚本自身 | 覆盖 `/usr/local/bin/rs`，先 `bash -n` |
| | 17 | 重签证书 | acme.sh |
| 内核管理 | 18 | 安装 / 更新内核 | `version.lock` 存在时按锁定版本装 |
| | 19 | 锁定 / 切换版本 | `version.lock`，`unlock` 解锁 |
| 进阶 | 20 | 诊断 | 六段检查 |
| | 21 | BBR | `/etc/sysctl.conf` |
| | 22 | 防火墙 | nftables / ufw / firewalld |
| | 23 | 卸载 | 可选连站点、证书、状态目录、防火墙一起清 |

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
├── rs                               # 单文件节点管理器 v2（23 项菜单，模板全内嵌）
├── install.sh                       # 多文件版安装脚本（读 conf/ 模板）
├── uninstall.sh                     # 多文件版卸载（--all / --purge）
├── scripts/
│   └── lint.sh                      # 本地自检：语法 / shellcheck / jq / 渲染 / 行尾
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

## 多客户端与订阅导出

面板没了，这些事交给 `rs` 的菜单：

**加客户端**（菜单 `2` 或命令行）：

```sh
rs add-client --email user2
# 输出：
# vless://<UUID>@<IP>:443?...&sni=api.example.com&...&pbk=<公钥>&sid=<shortId>#api-example-com-xxxxxx
```

`rs` 会往 `clients.conf` 追加一行、重渲染 `config.json`、`run -test` 通过后重启 xray，只断一次连接。

**看全部链接**（菜单 `3`）：

```sh
rs info      # 终端里列出每个客户端一条链接
rs export    # 同时写三个文件
```

```
/usr/local/etc/rs/export/links.txt     明文链接，一行一条
/usr/local/etc/rs/export/sub.txt       base64 订阅（可直接当订阅地址内容）
/usr/local/etc/rs/export/clash.yaml    Mihomo/Clash Meta，含 reality-opts
```

`clash.yaml` 片段（每加一个客户端自动多一条 proxy）：

```yaml
proxies:
  - name: "api.example.com-4f1c2a"
    type: vless
    server: 1.2.3.4
    port: 443
    uuid: 4f1c2a9e-...
    network: tcp
    tls: true
    udp: true
    servername: api.example.com
    client-fingerprint: chrome
    reality-opts:
      public-key: Qk3x...
      short-id: "0123456789abcdef"
    flow: xtls-rprx-vision
```

`xhttp` 模式下 `network: xhttp`、`xhttp-opts.path`，且不带 `flow`。

**删客户端**（菜单 `4`）：列出编号，选一个删掉，至少要留一个。

**改节点**（菜单 `5`）：域名 / 网络 / 回环端口 / 客户端连接地址；换网络时脚本自动同步每个客户端的 `flow`，换域名会删掉旧证书目录并触发重签。

**流量统计**（`rs` 默认不带面板，需要就自己开 Stats API）：

```json
"api": { "tag": "api", "services": [ "StatsService" ] },
"stats": {},
"policy": { "levels": { "0": { "statsUserUplink": true, "statsUserDownlink": true } } }
```

再加一个只监听回环的 inbound 与一条路由：

```json
{ "listen": "127.0.0.1", "port": 10085, "protocol": "dokodemo-door", "tag": "api",
  "settings": { "address": "127.0.0.1" } }
```

```json
{ "type": "field", "inboundTag": [ "api" ], "outboundTag": "api" }
```

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

用 `rs`（推荐）：

```sh
rs             # 菜单 [23] 卸载；会问要不要连站点、证书、状态目录、防火墙一起清
rs uninstall
```

用多文件版：

```sh
# 只删节点，Nginx 与站点保留
bash uninstall.sh --domain api.example.com

# 节点 + 站点 + 证书 + 防火墙
bash uninstall.sh --domain api.example.com --all

# 连 nginx / ufw / nftables 一起卸
bash uninstall.sh --domain api.example.com --purge
```

## 二次开发说明

本项目的菜单分层、状态文件组织、导出与诊断的划分方式，改写自
[0xdabiaoge/singbox-lite](https://github.com/0xdabiaoge/singbox-lite)（sing-box + Xray 多协议管理脚本，纯 Shell）。

| 部分 | 来源 |
|---|---|
| 单文件 `rs` 的主菜单分类（节点 / 服务 / 配置 / 内核 / 进阶）与编号式交互 | 参照 singbox-lite 改写的结构 |
| 状态与导出的落地方式（状态目录、链接清单、订阅、Clash 配置） | 参照 singbox-lite 的 `clash.yaml` 与元数据思路，按 REALITY 单协议重新实现 |
| 发行版 / init / 防火墙三套分支的判断方式 | 参照 singbox-lite 的多发行版适配思路改写 |
| REALITY 偷自己（dest 指向本机 Nginx）、回落链路、`serverNames` 与证书对齐、raw/xhttp 双传输 | 本项目自有实现 |
| Xray 配置渲染、密钥轮换、内核锁定、六段诊断、备份恢复 | 本项目自有实现 |

上游仓库当前未附带 `LICENSE` 文件；本仓库的 MIT 仅覆盖本项目自身代码，上游代码的再分发与商用请先行联系其作者。
本项目的差异点：只做 VLESS + REALITY 一条线（不引入 sing-box、不做多协议、不做 Argo 隧道、不做端口转发），
把「偷自己」这一种伪装做到配置、证书、诊断全链路闭环。
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
- [0xdabiaoge/singbox-lite](https://github.com/0xdabiaoge/singbox-lite) —— 本项目菜单与状态管理的改写来源
- [MHSanaei/3x-ui](https://github.com/MHSanaei/3x-ui) —— 面板方案的参照实现

## 参考

- REALITY 官方说明：<https://github.com/XTLS/REALITY>
- XHTTP 五合一配置讨论：<https://github.com/XTLS/Xray-core/discussions/4118>
- Xray 官方安装脚本：<https://github.com/XTLS/Xray-install>
