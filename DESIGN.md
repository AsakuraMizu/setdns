# setdns — Design

`setdns` 是一个 Rust library crate，用于临时应用并恢复 system DNS 配置，支持 global DNS 和 split DNS。第一版目标是 API 简单、行为可恢复、平台差异明确暴露为错误。

## API

```rust
use std::net::IpAddr;

pub struct Config {
    pub owner: String,
    pub servers: Vec<IpAddr>,
    pub domains: Vec<String>,
    pub device: Option<String>,
}

pub struct SetDns(Option<imp::SetDns>);

impl SetDns {
    pub fn apply(config: Config) -> Result<Self>;
    pub fn close(mut self) -> Result<()> {
        self.0.take().unwrap().close()
    }
}

impl Drop for SetDns {
    fn drop(&mut self) {
        if let Some(inner) = self.0.take() {
            if let Err(err) = inner.close() {
                log::warn!("failed to restore DNS configuration for setdns: {err}");
            }
        }
    }
}
```

### 生命周期语义

- `SetDns::apply(config)` 立即验证并应用 DNS 配置；成功返回时配置已经生效。
- `SetDns` 是持有恢复状态的 RAII handle。
- `close(self)` 消费 handle，执行确定性回收并返回错误。
- `Drop` 做 best-effort 回收；失败时用 `log::warn!` 记录错误。调用方需要处理错误时必须显式调用 `close()`。
- 不提供 `set()` / `clear()`。修改配置需要先 `close()`，再用新 `Config` 调 `SetDns::apply(config)`。
- `apply(config)` 会先清理同一 `owner` 的残留配置，再应用新配置。

### Config 字段

- `owner`：本库写入系统配置时使用的拥有者标识。用于文件 header、备份状态、Windows NRPT rule 标记等。
- `servers`：目标 DNS server。不能为空。不设置全局数量上限；遇到平台后端无法表示的数量时返回 `InvalidConfig`。已知限制：macOS `/etc/resolver` 使用 resolver(5)，`nameserver` 最多 `MAXNS` 个（当前为 3），只在 macOS split DNS 路径上强制。第一版使用 `std::net::IpAddr` 类型但仅 IPv4 路径经过验证；IPv6（含 link-local scope ID）和纯 IPv6 网络场景留待后续版本。
- `domains`：为空时表示 global DNS；非空时表示 split DNS。
- `device`：可选平台目标。
  - Linux：interface name，例如 `tun0` / `wg0`。内部用 `if_nametoindex` 转 ifindex。
  - macOS：BSD interface name，例如 `en0` / `en7` / `utun5`。不是 `networksetup` service name。
  - Windows：第一版忽略。

### 全局行为选择

- `domains.is_empty()`：global DNS。所有系统解析流量应使用 `servers`，但具体覆盖范围受平台后端限制。
- `domains` 非空：split DNS。只有匹配这些 suffix 的查询使用 `servers`，其余流量不动。
- 如果当前平台无法实现请求的模式，返回 `UnsupportedGlobalDns` 或 `UnsupportedSplitDns`。

### Domain 语义

`domains` 接受两类输入：

- `corp.internal`
- `*.corp.internal`

两者都表示 suffix match for `corp.internal`，同时匹配 apex 和子域：

- `corp.internal`
- `host.corp.internal`
- `a.b.corp.internal`

拒绝：

- 空字符串
- `.`
- `*`
- `.corp.internal`
- `corp.internal.`
- `*.`
- 非 ASCII 域名
- 含空 label 或非法字符的域名

内部规范化：

- ASCII 小写。
- `*.corp.internal` 记录为 `{ domain: "corp.internal", wildcard: true }`。
- `corp.internal` 记录为 `{ domain: "corp.internal", wildcard: false }`。

第一版不区分 wildcard 与普通 suffix 的底层行为，因为目标平台都提供 suffix/routing-domain 语义。`wildcard` 只保留原始意图，便于后续扩展和诊断。

### Domain coalescing

解析和规范化后，内部可以做低优先级 suffix coalescing：

- 去重：`corp.internal` 和 `*.corp.internal` 归一到同一个 suffix。
- 父 suffix 覆盖子 suffix：如果已保留 `corp.internal`，则 `dev.corp.internal`、`a.dev.corp.internal` 可丢弃。
- coalescing 只在所有 domains 共享同一组 `servers` 的当前 API 下安全。后续若支持不同 domain 使用不同 nameserver，必须重新评估。

示例：

```text
input:  ["*.corp.internal", "dev.corp.internal", "svc.company.net"]
output: ["corp.internal", "svc.company.net"]
```

coalescing 是优化，不是正确性前提。第一版可以先实现，因为逻辑小、能减少 macOS resolver 文件和 Windows NRPT namespace 数量；但不要为它引入复杂数据结构。

### owner 语义

`owner` 约束：

- ASCII。
- 长度 `1..=64`。
- 只允许 `[A-Za-z0-9._-]`。

原因：`owner` 会进入 root-owned 文件名、文件内容、备份状态、NRPT display/comment，不能接受任意字符串。

### Error

```rust
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid config: {0}")]
    InvalidConfig(String),

    #[error("split DNS is not supported by this backend")]
    UnsupportedSplitDns,

    #[error("global DNS is not supported by this backend")]
    UnsupportedGlobalDns,

    #[error("permission denied")]
    PermissionDenied,

    #[error("platform backend error: {0}")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync + 'static>),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}
```

规则：

- public `Error` 和各平台内部 error enum 都用 `thiserror` derive。
- `Backend` 保存真实 source error：`Box<dyn std::error::Error + Send + Sync + 'static>`。
- 不把后端错误压平成 `String`；需要上下文时在平台内部定义带上下文的 typed error，再 box 到 `Error::Backend`。

### 依赖

- `thiserror`：管理 public error 和平台内部 typed errors。
- `log`：只用于库内部诊断日志，例如 `Drop` 回收失败。第一版不用 `tracing`。

## 平台技术路线

### Linux

#### 行为选择

1. 连接 system bus，检测 `org.freedesktop.resolve1` 是否可用。
2. split DNS：必须有 `systemd-resolved`，且 `config.device` 必须指定 interface name。
3. global DNS：
   - `resolved` 可用且 `config.device` 存在 → 走 resolved link 配置。
   - `resolved` 不可用 → 走 direct `/etc/resolv.conf` 后端。
   - `resolved` 可用但 `config.device` 为空，且 `/etc/resolv.conf` 由 resolved 管理 → 返回 `UnsupportedGlobalDns`，避免破坏系统 DNS。

#### resolved 后端

- Crate：`zbus`，blocking API。
- Interface name → `if_nametoindex` → ifindex。
- D-Bus object：`org.freedesktop.resolve1`，path `/org/freedesktop/resolve1`。
- 调用：
  - `SetLinkDNS(ifindex, a(iay))`
  - `SetLinkDomains(ifindex, a(sb))`
  - `SetLinkDefaultRoute(ifindex, bool)`
  - `FlushCaches()`
- split DNS：
  - `corp.internal` / `*.corp.internal` → `(corp.internal., true)`。
  - `SetLinkDefaultRoute(false)`。
- global DNS：
  - 写 root route-only domain：`(".", true)`，等价于 `~.`。
  - `SetLinkDefaultRoute(true)`。
- 回收：`RevertLink(ifindex)`。
- 限制：`RevertLink` 会清掉该 link 上所有 resolved 设置。调用方必须传入自己拥有的 link，例如 tunnel interface。

#### direct `/etc/resolv.conf` 后端

- 仅用于 global DNS。
- 写入前备份原文件。
- 规范化写入流程（原子替换）：
  1. 备份原文件到 `/etc/resolv.conf.setdns.bak`。
  2. 写入带 owner header 的新内容到 `/etc/resolv.conf.setdns.tmp`。
  3. `fsync` 临时文件。
  4. `rename` 临时文件到 `/etc/resolv.conf`（原子操作）。
- `close()`：将备份文件 `rename` 回 `/etc/resolv.conf`，删除备份。
- 若 `/etc/resolv.conf` 由 resolved 管理（symlink 到 `/run/systemd/resolve/stub-resolv.conf` 或类似路径），不走 direct，返回 `UnsupportedGlobalDns`。
- 若 close 前文件已被外部改写（内容不含本库的 owner header），返回明确错误，不盲目覆盖用户/系统新内容。

### macOS

macOS 遵循上面的全局行为选择：global DNS 使用 SystemConfiguration 修改 service DNS；split DNS 使用 `/etc/resolver/<domain>`。

#### Global DNS

不要依赖 `networksetup` 输出。

原因：`networksetup -getdnsservers <service>` 只反映手工 DNS；当 DNS 来自 DHCP 时会返回 `There aren't any DNS Servers set on ...`，但有效 DNS 仍会出现在 `State:/Network/Service/<PrimaryService>/DNS` 和 `State:/Network/Global/DNS`。

核心理念：不调用 `SCPreferencesCommitChanges`。setdns 是临时 DNS 修改器，DNS 修改的生命周期绑定到 `SetDns` handle。Commit 会将配置持久化到磁盘，导致 DNS 修改存活过系统重启——但此时 VPN tunnel 已不存在，残留的 DNS 指向无效。Apply without Commit 使修改仅在 configd 内存中生效，系统重启后自动恢复原始 DNS，与 Linux resolved 后端行为一致。

实现使用 SystemConfiguration：

- `SCDynamicStore`：读取 `State:/Network/Global/IPv4`，取 `PrimaryService` 和 `PrimaryInterface`。
- `SCPreferences` / `SCNetworkService` / `SCNetworkProtocol`：修改 service DNS 配置。
- `system-configuration`：用于安全枚举 service、interface、service order。
- `system-configuration-sys`：用于低层 FFI：
  - `SCPreferencesLock`
  - `SCNetworkSetCopyCurrent`
  - `SCNetworkServiceCopyProtocol(..., kSCNetworkProtocolTypeDNS)`
  - `SCNetworkProtocolGetConfiguration`
  - `SCNetworkProtocolSetConfiguration`
  - `SCPreferencesApplyChanges`
  - `SCNetworkInterfaceForceConfigurationRefresh`

目标 service：

- `device = Some(ifname)`：在 current set 中按 service order 找第一个 enabled service，其 interface BSD name 等于 `ifname`。若同一 ifname 有多个匹配（罕见，如 AirPlay 2 虚拟接口），按 service order 优先级选择。
- `device = None`：读取 `State:/Network/Global/IPv4` 的 `PrimaryService`，只修改该 service。
- 不修改所有 enabled services。这样破坏面最小，恢复边界明确。

应用流程：

1. 定位目标 service。
2. 读取并保存完整原始 DNS protocol configuration 到内存（`imp::SetDns` 内部）。
3. 在原始 DNS dictionary 基础上写入新的 `ServerAddresses`，保留其他 key。
4. `SCPreferencesLock`。
5. `SCNetworkProtocolSetConfiguration`。
6. `SCPreferencesApplyChanges`。
7. `SCPreferencesUnlock`。
8. 必要时刷新 interface 配置。
9. best-effort `dscacheutil -flushcache`。

回收流程：

- `close()` 或 `Drop` 时：Lock → 从内存恢复原始 DNS protocol configuration → Apply → Unlock → best-effort flush cache。
- 若原始 DNS dictionary 不存在：清除 DNS protocol configuration，让 DHCP 重新接管。

Crash 行为：

- 修改仅在 configd 内存中，系统重启后自动恢复原始 DNS。
- Crash 后残留到 configd 重启为止，与 Linux resolved 后端风险一致。

不要写 `State:/Network/Global/DNS`。实测和资料都显示编辑 Global/DNS 不能可靠改变实际解析路径。

#### Split DNS

使用 `/etc/resolver/<domain>`，与 Tailscale 当前 macOS 路线一致。

文件内容：

```text
# Added by setdns (<owner>)
domain corp.internal
nameserver 10.0.0.1
nameserver 10.0.0.2
```

规则：

- `corp.internal` → `/etc/resolver/corp.internal`。
- `*.corp.internal` → `/etc/resolver/corp.internal`。
- 输入 domains 先 normalize / dedupe / coalesce；每个保留的 unique suffix 写一个 resolver 文件。
- `close()` 删除所有含 owner header 的 resolver 文件。
- `apply(config)` 先删除同 owner 残留 resolver 文件。
- 写入或删除后 best-effort `dscacheutil -flushcache`。
- macOS resolver(5) 的 `nameserver` 最多 `MAXNS` 个（当前 3）。split DNS 路径如果 `servers.len() > 3`，返回 `InvalidConfig`，不静默截断。

限制：macOS 26 有报告称 `/etc/resolver` 对 `.internal`、`.test`、`.home.arpa` 等 private/custom TLD 有回归，mDNSResponder 可能绕过 unicast resolver。这是系统行为，库只能记录限制并提供诊断错误。

#### 权限

global DNS 和 split DNS 都需要 root。权限错误向上传播。

### Windows

第一版直接调 PowerShell NRPT cmdlet；不修改 adapter DNS server。`device` 在 Windows 后端中忽略。

#### 应用流程

1. `SetDns::apply(config)` 清理同 owner 旧 rule。
2. 按全局行为选择生成 NRPT rule。
3. 记录 `Add-DnsClientNrptRule -PassThru` 返回的 `Name`。
4. `close()` 调 `Remove-DnsClientNrptRule -Name <name> -Force`。
5. 应用或清理后 best-effort `Clear-DnsClientCache`。

#### Global DNS

- 使用 NRPT 的 catch-all namespace：`.`。
- 命令形态：`Add-DnsClientNrptRule -Namespace "." -NameServers "10.0.0.1" -DisplayName <owner>`。
- 语义：经 Windows DNS Client 的查询都会匹配该 rule。
- 限制：绕过 Windows DNS Client 的应用不受 NRPT 影响。

#### Split DNS

- `corp.internal` → `.corp.internal`。
- `*.corp.internal` → `.corp.internal`。
- 命令形态：`Add-DnsClientNrptRule -Namespace ".corp.internal" -NameServers "10.0.0.1" -DisplayName <owner>`。

#### Domain chunking

NRPT 单 rule 的 Namespace 数组有长度限制。先对 domains 做 normalize / dedupe / coalesce，再按 Tailscale 参考实现每 50 个 namespace 一组拆分 rule。20 个左右的 split domains 不构成问题。所有 rule 共用同一组 nameserver。

#### PowerShell 调用方式

```rust
Command::new("powershell.exe")
    .args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-EncodedCommand",
        &encoded,
    ])
    .output()
```

- 将 script 编为 UTF-16LE，Base64 编码，传入 `-EncodedCommand`。
- 不拼 shell command string；通过脚本变量传参。
- PowerShell 失败时保留 stdout/stderr。

## 实现阶段

### Phase 1：公共类型与错误模型

文件：

- `Cargo.toml`
- `src/lib.rs`
- `src/error.rs`
- `src/config.rs`

内容：

- 定义 `Config`、`SetDns`、`Error`。
- 引入 `thiserror` 依赖；public `Error` 和平台内部 error enum 都用 `thiserror` derive。
- 引入 `log` 依赖；`Drop` 回收失败用 `log::warn!`。
- domain coalescing 实现在 config normalization 层，平台后端只接收 coalesced suffix list。
- 内部实现 owner/domain/server validation。
- `close(self)` 消费 handle。

验收：

- `cargo check` 通过。
- public API 只有 `Config`、`SetDns`、`Error` 和 `Result`。

### Phase 2：平台抽象

文件：

- `src/platform/mod.rs`
- `src/platform/unsupported.rs`
- `src/platform/linux/mod.rs`
- `src/platform/macos/mod.rs`
- `src/platform/windows/mod.rs`

内容：

- `imp::SetDns` 由 `cfg(target_os = ...)` 分发。
- 每个平台 handle 内部保存 close 所需状态。
- target-specific dependencies，Linux 构建不拉 macOS/Windows-only crate。

### Phase 3：Linux 后端

文件：

- `src/platform/linux/mod.rs`
- `src/platform/linux/resolved.rs`
- `src/platform/linux/resolv_conf.rs`

验收：

- resolved + `device` + split → route-only domains 写入。
- resolved + `device` + global → `~.` 写入。
- no resolved + global → resolv.conf 写入，`close()` 恢复。
- no resolved + split → `UnsupportedSplitDns`。

### Phase 4：macOS 后端

文件：

- `src/platform/macos/mod.rs`
- `src/platform/macos/global.rs`
- `src/platform/macos/resolver.rs`
- `src/platform/macos/state.rs`

验收：

- global：只修改 `device` 指定 interface 对应 service，或 `PrimaryService`。
- global：原始 DNS dictionary 完整恢复。
- split：`/etc/resolver/<domain>` 写入并清理。
- `*.corp.internal` 和 `corp.internal` 归一到同一 resolver 文件。

### Phase 5：Windows 后端

文件：

- `src/platform/windows/mod.rs`
- `src/platform/windows/powershell.rs`

验收：

- split：`corp.internal` / `*.corp.internal` → `Namespace .corp.internal`。
- global：`Namespace .`。
- `close()` 只删除本 owner 创建的 rule。
- PowerShell 失败保留完整诊断信息。

## 测试策略

第一阶段先不做系统集成测试。不要让 agent 写大量无用单元测试，尤其不要 mock D-Bus、PowerShell、SystemConfiguration、`networksetup` 或真实 DNS 解析路径；这些测试不会证明系统交互正确，只会固化实现细节。

第一阶段只接受少量纯解析/校验测试：

- owner validation：长度、字符集、空字符串。
- domain parsing / normalization / coalescing：
  - 接受 `corp.internal`、`*.corp.internal`。
  - 两者都归一为 suffix `corp.internal`，wildcard flag 分别保留。
  - 拒绝 `.corp.internal`、`corp.internal.`、`.`、`*`、`*.`、空 label、非 ASCII。
  - 去重 `corp.internal` / `*.corp.internal`。
  - parent suffix 覆盖 child suffix：`corp.internal` 覆盖 `dev.corp.internal`。

后续进入系统集成阶段时再补 Linux Docker、macOS root/manual、Windows admin/manual 测试。当前设计文档不要求 agent 提前实现这些测试。

### 日常检查

```sh
cargo +nightly fmt --all -- --check
cargo clippy --all-targets --all-features
```

## 关键风险

| 风险 | 影响 | 缓解 |
|------|------|------|
| resolved `RevertLink` 清空整条 link 的 DNS | 破坏非本库设置的 link DNS | 强制要求 caller 传 owned interface |
| direct resolv.conf 与 DHCP/NM 抢配置 | DNS 反复被覆盖 | 备份恢复 + owner header + trample 检测 |
| macOS global DNS 修改仅在 configd 内存中 | 进程 crash 后 DNS 修改残留到 configd 重启或系统重启为止 | 与 Linux resolved 后端行为一致；系统重启后自动恢复 |
| macOS `device=None` 只改 PrimaryService | 切换网络后新 primary service 不继承 DNS | 第一版保持最小破坏；后续再加 watcher |
| macOS `/etc/resolver` 在 macOS 26 private TLD 上可能失效 | `.internal` 等域名可能不走指定 nameserver | 文档记录系统限制；测试用 `scutil --dns` + 实际解析双验证 |
| split domains 很多 | macOS 写多个 `/etc/resolver` 文件；Windows NRPT rule 变长 | normalize / dedupe / coalesce；Windows 每 50 个 namespace chunk；20 个左右不是问题 |
| NRPT 不影响绕过 Windows DNS Client 的应用 | 部分应用流量不走 NRPT | 文档记录限制，不做无谓补偿 |
| PowerShell 调用开销 | set/close 速度慢 | 第一版可接受；后续改 registry/WMI |
| IPv6 未覆盖 | 纯 IPv6 网络或 link-local IPv6 DNS server 可能无法工作 | 第一版暂不支持；`servers` 字段接受 `IpAddr` 但仅 IPv4 路径经过验证；IPv6 支持后续添加 |

## 参考

- [org.freedesktop.resolve1](https://www.freedesktop.org/software/systemd/man/latest/org.freedesktop.resolve1.html)
- [systemd-resolved routing](https://www.freedesktop.org/software/systemd/man/latest/systemd-resolved.service.html)
- [System Configuration Framework Components](https://developer.apple.com/library/archive/documentation/Networking/Conceptual/SystemConfigFrameworks/SC_Components/SC_Components.html)
- [macOS resolver(5)](https://www.manpagez.com/man/5/resolver/)
- [system-configuration crate](https://docs.rs/system-configuration/latest/system_configuration/)
- [system-configuration-sys network configuration](https://docs.rs/system-configuration-sys/latest/system_configuration_sys/network_configuration/)
- [Add-DnsClientNrptRule](https://learn.microsoft.com/en-us/powershell/module/dnsclient/add-dnsclientnrptrule)
- [Get-DnsClientNrptRule](https://learn.microsoft.com/en-us/powershell/module/dnsclient/get-dnsclientnrptrule)
- [Remove-DnsClientNrptRule](https://learn.microsoft.com/en-us/powershell/module/dnsclient/remove-dnsclientnrptrule)
- [DnsClientNrptRule class](https://learn.microsoft.com/en-us/previous-versions/windows/desktop/dnsclientpsprov/dnsclientnrptrule)
- [Tailscale DNS](https://github.com/tailscale/tailscale/tree/main/net/dns)
- [Tailscale macOS DNS resolver approach (PR #18272)](https://github.com/tailscale/tailscale/pull/18272)
- [Tailscale macOS DynamicStore GlobalProtect conflict (Issue #9243)](https://github.com/tailscale/tailscale/issues/9243)
