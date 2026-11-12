# CHANGELOG

## 2.0.0

`rs` 从「一次性安装器」升级成「节点管理器」，菜单分层与状态组织参照 [0xdabiaoge/singbox-lite](https://github.com/0xdabiaoge/singbox-lite) 改写。

### 新增

- 23 项主菜单：节点管理 / 服务控制 / 配置更新 / 内核管理 / 进阶。
- 多客户端：`clients.conf` 一行一个 UUID，增删后自动重渲染配置、`run -test`、重启。
- 订阅导出：`links.txt`、`sub.txt`（base64）、`clash.yaml`（Mihomo `reality-opts`）。
- 修改节点：域名 / 网络 / 回环端口 / 客户端连接地址，换网络时同步 `flow`，换域名触发重签。
- 轮换 REALITY 密钥：一条命令刷新 `x25519`，旧链接立即作废。
- 内核治理：最新版、`--xray-version` 指定、`version.lock` 锁定 / `unlock` 解锁。
- 备份恢复：打包 `xray` 配置、状态目录、证书、service 文件；恢复后自动重载。
- 六段诊断：DNS 对齐 / 端口归属 / 本机回落 / 外部 443 真证书 / 证书余期 / 配置与服务。
- 定时重启：Alpine 写 `/etc/crontabs/root`，其余写 `/etc/cron.d/rs-xray`。
- BBR 开关、防火墙重建 / 关闭、证书重签。
- 状态目录 `/usr/local/etc/rs/`（700 / 600），兼容从旧版 `/root/reality-selfsteal.info` 导入。

### 变更

- `rs install` 支持 `--no-cert` / `--no-firewall` / `-y`。
- 客户端语法校验前移到 root 检查之前，非 root 也能得到准确的参数错误。
- `xhttp` 下渲染配置时不再写 `flow`（Vision 只配 raw/tcp）。
- `scripts/lint.sh` 增加内嵌模板渲染检查（raw / xhttp / nginx 两个分支 / 多客户端）。

### 修复

- `add-client` 不再刷新服务端 `x25519` 密钥（旧版会让已有客户端掉线）。
- `links.txt` 每条链接独立成行；`clash.yaml` 的 `network` 输出 `tcp` / `xhttp` 而非 `raw`。

## 1.0.0

- 单文件 `rs`：安装 / 信息 / 自检 / 升级 / 卸载 / 自更新。
- 多文件版 `install.sh` + `conf/` 模板 + `lib/common.sh`。
- 跨发行版：Alpine（apk / OpenRC / nftables）、Debian/Ubuntu（apt / systemd / ufw）、RHEL 系（dnf / firewalld）。
- GitHub Actions：`bash -n`、严格 shellcheck、jq 校验模板、`rs` CLI 冒烟、Alpine 3.21 容器任务。