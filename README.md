# setdns

`setdns` 临时修改系统 DNS，句柄关闭或丢弃时恢复原状态。支持全局 DNS 和按域名后缀分流的 split DNS。

典型用途：VPN、隧道、测试环境、企业内网域名解析，或任何需要接管系统解析器一段时间的工具。

## 快速开始

```rust,no_run
use std::net::IpAddr;

use setdns::{Config, SetDns};

fn main() -> setdns::Result<()> {
    let dns = SetDns::apply(Config {
        owner: "example".to_owned(),
        servers: vec!["1.1.1.1".parse::<IpAddr>().expect("valid IP")],
        domains: Vec::new(),
        device: Some("tun0".to_owned()),
    })?;

    // Run the work that needs the temporary DNS settings here.

    dns.close()
}
```

`close` 恢复原配置，返回恢复错误。若直接丢弃句柄，`Drop` 也尝试恢复；因为 `Drop` 不能返回错误，恢复失败仅通过 `log` 记录。

## 保留配置

若需进程退出后保留 DNS 配置，在 `SetDns::apply` 成功后调用 `std::mem::forget` 跳过 `Drop` 恢复：

```rust,no_run
use std::{mem, net::IpAddr};

use setdns::{Config, SetDns};

fn main() -> setdns::Result<()> {
    let dns = SetDns::apply(Config {
        owner: "vpn.example".to_owned(),
        servers: vec!["10.0.0.53".parse::<IpAddr>().expect("valid IP")],
        domains: vec!["corp.internal".to_owned()],
        device: Some("tun0".to_owned()),
    })?;

    mem::forget(dns);
    Ok(())
}
```

`forget` 的代价：丢失 `close` 能返回的恢复错误，原始状态也不再保留在内存中。下次进程启动后，唯有用相同 `owner` 再次调用 `SetDns::apply`，后端才有机会清理旧状态。

以下后端按 `owner` 清理旧状态：Linux `/etc/resolv.conf`、macOS `/etc/resolver` split DNS、Windows NRPT。systemd-resolved link DNS、macOS 全局 DNS 和 Windows 全局 DNS 的接口设置不保存跨进程可恢复的原始状态——对它们来说，`forget` 就是把系统 DNS 改成新值，何时改回由调用方决定。

## 配置

`Config` 描述一次临时 DNS 修改：

| 字段      | 含义                                                                                                           |
| --------- | -------------------------------------------------------------------------------------------------------------- |
| `owner`   | 写入系统状态时使用的所有者标记。1–64 字节 ASCII 字符串，只能包含字母、数字、`.`、`_` 和 `-`。                  |
| `servers` | 句柄存活期间使用的 DNS 服务器。不能为空。                                                                      |
| `domains` | 为空表示全局 DNS；非空表示 split DNS。域名必须是 ASCII DNS 后缀。`*.corp.internal` 按 `corp.internal` 处理。   |
| `device`  | 可选的平台目标，通常是网络接口名。各平台行为见下表。                                                           |

split DNS 示例：

```rust,no_run
use std::net::IpAddr;

use setdns::{Config, SetDns};

fn main() -> setdns::Result<()> {
    let dns = SetDns::apply(Config {
        owner: "vpn.example".to_owned(),
        servers: vec!["10.0.0.53".parse::<IpAddr>().expect("valid IP")],
        domains: vec!["corp.internal".to_owned(), "dev.corp.internal".to_owned()],
        device: Some("tun0".to_owned()),
    })?;

    dns.close()
}
```

配置先标准化再传给平台后端：域名转小写，重复后缀去重，被父后缀覆盖的子后缀省略。

## 平台行为

### Linux

全局 DNS 有两条路径：传入 `device` 且 systemd-resolved 可用时，配置该接口的 link DNS；否则在 `/etc/resolv.conf` 未由 systemd-resolved 管理时，直接替换并备份 `/etc/resolv.conf`。

split DNS 需要 systemd-resolved，也必须传入 `device`。`device` 作为接口名传给 systemd-resolved。

`/etc/resolv.conf` 后端创建 `/etc/resolv.conf.setdns.bak`，仅当当前文件仍带有同一 `owner` 标记时才恢复，避免覆盖其他进程写入的配置。

### macOS

全局 DNS 通过 SystemConfiguration 修改目标 network service。传入 `device` 时按 BSD 接口名查找 service；动态 `utun*` 接口找不到 service 则回退到 primary service。

split DNS 在 `/etc/resolver` 下为每个后缀写 resolver 文件，忽略 `device`。macOS resolver 文件每个配置最多 3 个 nameserver。

### Windows

全局 DNS 通过 DNS Client NRPT 规则接管解析；传入 `device` 时，Windows 还会通过 `SetInterfaceDnsSettings` 设置该接口的 DNS 服务器。split DNS 只写按后缀匹配的 NRPT 规则，忽略 `device`。

### 其他平台

其他平台返回不支持错误，不修改系统 DNS。

## 权限与错误处理

修改系统 DNS 通常需要管理员权限：Linux 写 `/etc/resolv.conf` 或访问 systemd-resolved 系统 D-Bus，macOS 改 SystemConfiguration 或 `/etc/resolver`，Windows 写 DNS Client NRPT 注册表规则，且在全局 DNS + `device` 模式下修改接口 DNS 设置。

错误分两类：`Error::InvalidConfig`——配置在进入平台代码前被拒绝；`Error::Backend`——包装系统 API、权限、I/O 或平台不支持错误。

## 日志

库通过 `log` crate 记录后端选择、应用、恢复和 `Drop` 恢复失败。调用方接入 `env_logger`、`tracing-log` 或其他 `log` 实现即可。
