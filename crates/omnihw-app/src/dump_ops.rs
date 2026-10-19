//! Reading crash dumps. `C:\Windows\Minidump` is readable by administrators only, so unless
//! Omni-HW already runs elevated, a short-lived elevated copy reads them
//! (`--read-dumps <result file>`), writes the analysis to the result file and exits without
//! opening a window, like the helpers in `power_ops` and `net_ops`. Only ever runs when the
//! user clicks "Analyze crash dumps". Blocks, so it runs on a worker thread (see
//! `Sampler::read_dumps`).

use crate::power_ops::{helper_result_path, quote};
use omnihw_core::minidump::{self, DumpAnalysis, MAX_DUMPS};
use std::path::PathBuf;

const READ_FLAG: &str = "--read-dumps";
/// Result files live in %TEMP% with this prefix; the helper refuses any other path.
const FILE_PREFIX: &str = "omnihw-dumps-";

/// If this process was started as the elevated dump reader, do the job and return the exit
/// code.
pub fn run_helper() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    helper(&args, read_direct)
}

/// The helper's logic, with the read step passed in so tests never touch the dump folder.
fn helper(
    args: &[String],
    read: impl FnOnce() -> Result<Vec<DumpAnalysis>, String>,
) -> Option<i32> {
    match args {
        [flag, out] if flag == READ_FLAG => {
            let Some(out) = helper_result_path(out, FILE_PREFIX) else {
                return Some(2);
            };
            let (code, text) = match read() {
                Ok(d) => (0, minidump::to_text(&d)),
                Err(e) => (1, e),
            };
            let _ = std::fs::write(out, text);
            Some(code)
        }
        [flag, ..] if flag == READ_FLAG => Some(2),
        _ => None,
    }
}

fn read_direct() -> Result<Vec<DumpAnalysis>, String> {
    minidump::read_folder(&minidump::dump_folder(), MAX_DUMPS)
}

fn temp_file() -> PathBuf {
    std::env::temp_dir().join(format!("{FILE_PREFIX}{}.txt", std::process::id()))
}

/// Analyse the newest crash dumps, asking for administrator rights when needed. No folder
/// means no dumps, without a prompt. Blocks.
pub fn read_dumps() -> Result<Vec<DumpAnalysis>, String> {
    // Only a folder known to be missing skips the prompt (not one we may not look at).
    if crate::elevate::elevated() || matches!(minidump::dump_folder().try_exists(), Ok(false)) {
        return read_direct();
    }
    let out = temp_file();
    let code = crate::elevate::run_as_admin_wait(&format!("{READ_FLAG} {}", quote(&out)));
    let text = std::fs::read_to_string(&out).unwrap_or_default();
    let _ = std::fs::remove_file(&out);
    match code? {
        0 => Ok(minidump::from_text(&text)),
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
    fn helper_validates_its_result_path() {
        let out = temp_file().with_file_name("omnihw-dumps-test-helper.txt");
        let out_s = out.to_string_lossy().to_string();
        let never = || -> Result<Vec<DumpAnalysis>, String> { panic!("must not read") };

        // Not the helper.
        assert_eq!(helper(&args(&["--no-elevate"]), never), None);
        assert_eq!(helper(&[], never), None);
        // Wrong argument count, or a result file outside %TEMP% or without the prefix.
        assert_eq!(helper(&args(&[READ_FLAG]), never), Some(2));
        assert_eq!(helper(&args(&[READ_FLAG, &out_s, "x"]), never), Some(2));
        assert_eq!(
            helper(&args(&[READ_FLAG, r"C:\Windows\omnihw-dumps-x.txt"]), never),
            Some(2)
        );
        for name in ["omnihw-net-x.txt", "omnihw-dumps-x.dll"] {
            let evil = std::env::temp_dir().join(name);
            assert_eq!(
                helper(&args(&[READ_FLAG, &evil.to_string_lossy()]), never),
                Some(2)
            );
        }
        // Results are written for the app to decode; errors are reported.
        let d = DumpAnalysis {
            file: "a.dmp".into(),
            code: 0x133,
            ..Default::default()
        };
        let code = helper(&args(&[READ_FLAG, &out_s]), || Ok(vec![d.clone()]));
        assert_eq!(code, Some(0));
        let text = std::fs::read_to_string(&out).unwrap_or_default();
        assert_eq!(minidump::from_text(&text), vec![d]);
        let code = helper(&args(&[READ_FLAG, &out_s]), || Err("denied".into()));
        assert_eq!(code, Some(1));
        assert_eq!(std::fs::read_to_string(&out).unwrap_or_default(), "denied");
        let _ = std::fs::remove_file(&out);
    }
}
