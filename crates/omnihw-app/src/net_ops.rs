//! Applying network changes. Everything but flushing the DNS cache needs administrator rights,
//! so unless Omni-HW already runs elevated, a short-lived elevated copy does the one change
//! (`--net-action <hex> <result file>`) and exits without opening a window, like the power
//! helper in `power_ops`. The helper decodes and validates the action again before acting.
//! Blocks, so it runs on a worker thread (see `Sampler::net_action`).

use crate::power_ops::{helper_result_path, quote};
use omnihw_core::network::{self, NetAction};
use std::path::PathBuf;

const ACTION_FLAG: &str = "--net-action";
/// Result files live in %TEMP% with this prefix; the helper refuses any other path.
const FILE_PREFIX: &str = "omnihw-net-";

/// If this process was started as the elevated network helper, do the job and return the
/// exit code.
pub fn run_helper() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    helper(&args, network::apply)
}

/// The helper's logic, with the apply step passed in so tests never touch the network.
fn helper(args: &[String], apply: impl FnOnce(&NetAction) -> Result<(), String>) -> Option<i32> {
    match args {
        [flag, arg, out] if flag == ACTION_FLAG => {
            let Some(out) = helper_result_path(out, FILE_PREFIX) else {
                return Some(2);
            };
            let (code, text) = match NetAction::from_arg(arg) {
                Some(a) => match apply(&a) {
                    Ok(()) => (0, String::new()),
                    Err(e) => (1, e),
                },
                None => (1, "Invalid network change".into()),
            };
            let _ = std::fs::write(out, text);
            Some(code)
        }
        [flag, ..] if flag == ACTION_FLAG => Some(2),
        _ => None,
    }
}

fn temp_file() -> PathBuf {
    std::env::temp_dir().join(format!("{FILE_PREFIX}action-{}.txt", std::process::id()))
}

/// Apply a change, asking for administrator rights when needed. Blocks.
pub fn apply(action: &NetAction) -> Result<(), String> {
    action.validate()?;
    if !action.needs_admin() || crate::elevate::elevated() {
        return network::apply(action);
    }
    let out = temp_file();
    let code = crate::elevate::run_as_admin_wait(&format!(
        "{ACTION_FLAG} {} {}",
        action.to_arg(),
        quote(&out)
    ));
    let text = std::fs::read_to_string(&out).unwrap_or_default();
    let _ = std::fs::remove_file(&out);
    match code? {
        0 => Ok(()),
        _ if !text.is_empty() => Err(text),
        c => Err(format!("the elevated helper failed (code {c})")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn helper_validates_before_acting() {
        let out = temp_file().with_file_name("omnihw-net-test-helper.txt");
        let out_s = out.to_string_lossy().to_string();
        let never = |_: &NetAction| -> Result<(), String> { panic!("must not apply") };

        // Not the helper.
        assert_eq!(helper(&args(&["--no-elevate"]), never), None);
        assert_eq!(helper(&[], never), None);
        // Wrong argument count or a result file outside %TEMP%.
        assert_eq!(helper(&args(&[ACTION_FLAG, "00"]), never), Some(2));
        let flush = NetAction::FlushDns.to_arg();
        assert_eq!(
            helper(
                &args(&[ACTION_FLAG, &flush, r"C:\Windows\omnihw-net-x.txt"]),
                never
            ),
            Some(2)
        );
        let evil = std::env::temp_dir().join("omnihw-power-x.txt");
        assert_eq!(
            helper(
                &args(&[ACTION_FLAG, &flush, &evil.to_string_lossy()]),
                never
            ),
            Some(2)
        );
        // Malformed or invalid actions are refused without applying.
        let bad = NetAction::Mtu {
            if_index: 7,
            name: "x".into(),
            mtu: 10,
        }
        .to_arg();
        for a in ["zz", "", bad.as_str()] {
            assert_eq!(helper(&args(&[ACTION_FLAG, a, &out_s]), never), Some(1));
        }
        assert_eq!(
            std::fs::read_to_string(&out).unwrap_or_default(),
            "Invalid network change"
        );
        // A valid action reaches `apply`, and its error is reported.
        let mut seen = None;
        let code = helper(&args(&[ACTION_FLAG, &flush, &out_s]), |a| {
            seen = Some(a.clone());
            Err("nope".into())
        });
        assert_eq!((code, seen), (Some(1), Some(NetAction::FlushDns)));
        assert_eq!(std::fs::read_to_string(&out).unwrap_or_default(), "nope");
        let _ = std::fs::remove_file(&out);
    }
}
