//! 凭据库可用性探针。
//!
//! 跑法：cargo run -p rustrss-desktop --example keyring_probe
//!
//! 为什么单独做一个探针：API key 的存储方案完全押在操作系统凭据库上，
//! 而 Linux 下它依赖 Secret Service（KWallet / GNOME Keyring）——「装了桌面环境」
//! 不等于「这个接口可用」。先测，再决定要不要做降级方案。

fn main() {
    const SERVICE: &str = "rustrss-probe";
    const ACCOUNT: &str = "self-test";

    println!("平台: {}", std::env::consts::OS);

    let entry = match keyring::Entry::new(SERVICE, ACCOUNT) {
        Ok(e) => e,
        Err(e) => {
            println!("结果: 无法创建凭据条目 → {e}");
            std::process::exit(1);
        }
    };

    let secret = "probe-value-请不要在意这个值";
    if let Err(e) = entry.set_password(secret) {
        println!("写入失败: {e}");
        println!("结果: 凭据库不可用（需要 Secret Service / KWallet 之类的后端）");
        std::process::exit(1);
    }
    println!("写入: ok");

    match entry.get_password() {
        Ok(got) if got == secret => println!("读回: ok（值一致）"),
        Ok(got) => {
            println!("读回: 值不一致，拿到 {got:?}");
            std::process::exit(1);
        }
        Err(e) => {
            println!("读回失败: {e}");
            std::process::exit(1);
        }
    }

    match entry.delete_credential() {
        Ok(()) => println!("清理: ok（探针未留下残留）"),
        Err(e) => println!("清理失败（不影响结论）: {e}"),
    }
    println!("结果: 凭据库可用 ✓");
}
