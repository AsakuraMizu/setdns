# setdns

临时应用并恢复系统 DNS 配置，支持全局 DNS 和按域名后缀分流的 split DNS。

## Usage

```rust,no_run
use std::net::IpAddr;

use setdns::{Config, SetDns};

fn main() -> setdns::Result<()> {
    let handle = SetDns::apply(Config {
        owner: "example".to_owned(),
        servers: vec!["1.1.1.1".parse::<IpAddr>().expect("valid IP")],
        domains: vec!["corp.internal".to_owned()],
        device: Some("tun0".to_owned()),
    })?;

    handle.close()
}
```

调用 `close` 恢复先前配置；如果句柄被丢弃，库也会尝试恢复并记录错误。

## Configuration

- `owner` 标记本库写入的系统 DNS 状态，必须是 1 到 64 字节的 ASCII 标识符。
- `servers` 是应用期间使用的 DNS 服务器列表。
- `domains` 为空时配置全局 DNS；非空时按 DNS 后缀配置 split DNS。
- `device` 是可选平台目标，通常是网络接口名。

## Platform notes

- Linux split DNS 需要 systemd-resolved 和 `device`；全局 DNS 可使用 `/etc/resolv.conf` 或 systemd-resolved。
- macOS 全局 DNS 使用 SystemConfiguration。`device` 会按 BSD 接口名定位 network service；动态 `utun*` 接口找不到 service 时回退到 primary service。split DNS 使用 `/etc/resolver` 并忽略 `device`。
- Windows 使用 PowerShell 配置 NRPT 规则，并忽略 `device`。
