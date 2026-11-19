//! Windows Firewall: makes sure Raketo may receive connections on every
//! network profile before the LAN server opens its port, so Windows never
//! shows its "allow access" prompt and no manual setup is needed.
//!
//! The installer creates the rule silently. A portable copy fixes it itself
//! the first time sharing starts, with a single elevation prompt.

use std::os::windows::process::CommandExt;
use std::process::Command;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// NET_FW_PROFILE2_DOMAIN | PRIVATE | PUBLIC
const ALL_PROFILES: i64 = 0x7;

fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Inbound rules for this executable: (has a block rule, profiles allowed).
fn inspect(exe: &str) -> Option<(bool, i64)> {
    let script = format!(
        "$p={};$fw=New-Object -ComObject HNetCfg.FwPolicy2;$b=0;$m=0;\
         foreach($r in $fw.Rules){{ if($r.Direction -eq 1 -and $r.Enabled -and $r.ApplicationName -and ($r.ApplicationName -ieq $p)){{ \
         if($r.Action -eq 0){{$b=1}} else {{$m=$m -bor $r.Profiles}} }} }};\"$b $m\"",
        ps_quote(exe)
    );
    let out = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut it = text.split_whitespace();
    let block = it.next()?.parse::<i64>().ok()? != 0;
    let mask = it.next()?.parse::<i64>().ok()?;
    Some((block, mask))
}

fn is_ok(exe: &str) -> bool {
    matches!(inspect(exe), Some((false, m)) if m & ALL_PROFILES == ALL_PROFILES)
}

/// Replaces this program's rules with one inbound allow rule, elevated.
fn fix(exe: &str) -> Result<(), String> {
    use windows::core::{w, HSTRING, PCWSTR};
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
    use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

    let q = exe.replace('"', "");
    let cmd = format!(
        "/c netsh advfirewall firewall delete rule name=all program=\"{q}\" & \
         netsh advfirewall firewall add rule name=\"Raketo\" dir=in action=allow program=\"{q}\" enable=yes profile=any \
         description=\"Raketo: share the printer on the local network\""
    );
    let params = HSTRING::from(cmd);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"),
        lpFile: w!("cmd.exe"),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    unsafe {
        // Fails when the user declines the elevation prompt.
        ShellExecuteExW(&mut info).map_err(|e| e.message().to_string())?;
        if !info.hProcess.is_invalid() {
            WaitForSingleObject(info.hProcess, INFINITE);
            let mut code = 0u32;
            GetExitCodeProcess(info.hProcess, &mut code).ok();
            CloseHandle(info.hProcess).ok();
        }
    }
    Ok(())
}

/// Returns `true` when inbound connections are allowed on every profile.
pub fn ensure() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let exe = exe.to_string_lossy().into_owned();
    if is_ok(&exe) {
        return true;
    }
    if fix(&exe).is_err() {
        return false;
    }
    is_ok(&exe)
}
