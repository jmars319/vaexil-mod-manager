//! A stand-in for SMF's `Deploy.exe`, for the smoke test only. It prints a
//! few lines shaped like SMF's own, then exits with `FAKE_DEPLOY_EXIT` (0 when
//! unset). It reads and writes nothing.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    println!("INFO\tDeploy\tInitialising error reporting ({})", args.join(" "));
    std::thread::sleep(std::time::Duration::from_millis(400));
    println!("INFO\tDeploy\tStaging RPKG mod");
    eprintln!("WARN\tDeploy\tA warning on stderr");
    std::thread::sleep(std::time::Duration::from_millis(400));
    println!("INFO\tDeploy\tDone in 1s");
    let code = std::env::var("FAKE_DEPLOY_EXIT").ok().and_then(|c| c.parse().ok()).unwrap_or(0);
    std::process::exit(code);
}
