//! SigmaOS-boot-manager — bootctl/efibootmgr-style CLI.
//!
//! Usage:
//!   sigmaos-boot list|status|entries
//!   sigmaos-boot add <id> <title> <kernel> [initrd] [cmdline...]
//!   sigmaos-boot remove <id>
//!   sigmaos-boot default <id>
//!   sigmaos-boot boot <id>          (simulate boot attempt / boot counting)
//!   sigmaos-boot success <id>       (mark last boot successful)
//!   sigmaos-boot hide|unhide <id>
//!   sigmaos-boot timeout <secs>

use sigmaos_boot_manager::{BootEntry, SigmaOSBootManager};
use std::process::exit;

fn usage() -> ! {
    eprintln!("usage: sigmaos-boot <list|status|entries|add|remove|default|boot|success|hide|unhide|timeout> [args]");
    exit(2);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() { usage(); }
    let state = SigmaOSBootManager::state_path();
    let mut m = SigmaOSBootManager::load_from(&state).unwrap_or_else(|_| SigmaOSBootManager::seeded());
    let cmd = args[0].as_str();
    let rest = &args[1..];
    let arg = |i: usize| -> &str { rest.get(i).map(|s| s.as_str()).unwrap_or_else(|| usage()) };

    let result: Result<(), String> = (|| -> Result<(), String> { match cmd {
        "status" => { println!("{}", m.status_line()); Ok(()) }
        "list" => {
            for e in m.entries() {
                let mark = if Some(e.id.as_str()) == m.config().default_entry.as_deref() { "*" } else { " " };
                println!("{} {:<18} {:<10} {:<28} tries={}", mark, e.id, e.os.label(), e.title, e.boot_count);
            }
            Ok(())
        }
        "entries" => { print!("{}", m.render_entries()); Ok(()) }
        "add" => {
            let id = arg(0); let title = arg(1); let kernel = arg(2);
            let mut e = BootEntry::linux(id, title, kernel);
            if let Some(initrd) = rest.get(3) { if !initrd.starts_with('-') { e = e.with_initrd(initrd); } }
            let cl: Vec<&str> = rest.iter().skip(3).filter(|s| s.contains('=')).map(|s| s.as_str()).collect();
            if !cl.is_empty() { e = e.with_cmdline(cl.join(" ")); }
            m.add(e).map(|_| println!("added {id}")).map_err(|e| e.to_string())
        }
        "remove" => { let id = arg(0); m.remove(id).map(|_| println!("removed {id}")).map_err(|e| e.to_string()) }
        "default" => { let id = arg(0); m.set_default(id).map(|_| println!("default={id}")).map_err(|e| e.to_string()) }
        "boot" => {
            let id = arg(0);
            let left = m.boot_attempt(id).map_err(|e| e.to_string())?;
            println!("booting {id} (tries left: {left})"); Ok(())
        }
        "success" => { let id = arg(0); m.mark_boot_success(id).map(|_| println!("{id} marked boot-success")).map_err(|e| e.to_string()) }
        "hide" | "unhide" => {
            let id = arg(0);
            let e = m.get_mut(id).ok_or_else(|| format!("no entry {id}"))?;
            e.hidden = cmd == "hide";
            println!("{} {}", if e.hidden { "hid" } else { "unhid" }, id); Ok(())
        }
        "timeout" => {
            let t: u32 = arg(0).parse().map_err(|_| "bad secs")?;
            m.config_mut().timeout_secs = t;
            println!("timeout={t}s"); Ok(())
        }
        _ => usage(),
    } })();

    match result {
        Ok(()) => { if let Err(e) = m.save_to(&state) { eprintln!("sigmaos-boot: warn: save: {e}"); } }
        Err(e) => { eprintln!("sigmaos-boot: {e}"); exit(1); }
    }
}
