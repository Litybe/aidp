//! Unity status of a project: the editor process, the lock, the latest compile and playtest results.

use crate::sync::lock;
use crate::unity_lock::ResourceHolder;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

/// How long a process listing is reused.
pub const PROCESS_CACHE_TTL: Duration = Duration::from_secs(5);
const PROJECT_PATH_FLAG: &str = "-projectpath";
const COMPILE_RESULTS: [&str; 3] = [".aidp", "compile", "results"];
const PLAYTEST_RESULTS: [&str; 3] = [".aidp", "playtest", "results"];
const HEARTBEAT_FILE: [&str; 3] = [".aidp", "editor", "heartbeat.json"];

/// A running Unity editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnityProcess {
    pub pid: u32,
    pub command_line: String,
}

/// The Unity editor processes of the machine.
pub trait ProcessSource: Send + Sync {
    fn unity_processes(&self) -> Vec<UnityProcess>;
}

/// Lists processes through the operating system: PowerShell CIM on Windows, `ps` elsewhere.
pub struct SystemProcesses;

impl ProcessSource for SystemProcesses {
    fn unity_processes(&self) -> Vec<UnityProcess> {
        system_processes()
    }
}

#[cfg(windows)]
fn system_processes() -> Vec<UnityProcess> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const SCRIPT: &str = "Get-CimInstance Win32_Process -Filter \"Name='Unity.exe'\" | \
         Select-Object ProcessId,CommandLine | ConvertTo-Json -Compress";
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    match output {
        Ok(output) => parse_cim_json(&String::from_utf8_lossy(&output.stdout)),
        Err(error) => {
            tracing::warn!(%error, "unity process listing failed");
            Vec::new()
        }
    }
}

#[cfg(not(windows))]
fn system_processes() -> Vec<UnityProcess> {
    let output = std::process::Command::new("ps")
        .args(["-eo", "pid=,args="])
        .output();
    match output {
        Ok(output) => parse_ps(&String::from_utf8_lossy(&output.stdout)),
        Err(error) => {
            tracing::warn!(%error, "unity process listing failed");
            Vec::new()
        }
    }
}

/// `ConvertTo-Json` output of `Win32_Process`: nothing, one object or an array of objects.
pub fn parse_cim_json(text: &str) -> Vec<UnityProcess> {
    let Ok(value) = serde_json::from_str::<Value>(text.trim()) else {
        return Vec::new();
    };
    let items = match value {
        Value::Array(items) => items,
        object @ Value::Object(_) => vec![object],
        _ => Vec::new(),
    };
    items
        .iter()
        .filter_map(|item| {
            Some(UnityProcess {
                pid: u32::try_from(item.get("ProcessId")?.as_u64()?).ok()?,
                command_line: item.get("CommandLine")?.as_str()?.to_string(),
            })
        })
        .collect()
}

/// `ps -eo pid=,args=` lines whose command is a Unity editor.
pub fn parse_ps(text: &str) -> Vec<UnityProcess> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let (pid, command) = line.split_once(char::is_whitespace)?;
            let program = command.split_whitespace().next()?;
            let name = program.rsplit('/').next().unwrap_or(program);
            if !name.eq_ignore_ascii_case("unity") {
                return None;
            }
            Some(UnityProcess {
                pid: pid.parse().ok()?,
                command_line: command.trim().to_string(),
            })
        })
        .collect()
}

/// A process listing reused for [`PROCESS_CACHE_TTL`].
pub struct CachedProcesses {
    source: Box<dyn ProcessSource>,
    ttl: Duration,
    cache: Mutex<Option<(Instant, Vec<UnityProcess>)>>,
}

impl CachedProcesses {
    pub fn new(source: Box<dyn ProcessSource>, ttl: Duration) -> Self {
        CachedProcesses {
            source,
            ttl,
            cache: Mutex::new(None),
        }
    }

    pub fn list(&self) -> Vec<UnityProcess> {
        let mut cache = lock(&self.cache);
        if let Some((at, processes)) = cache.as_ref() {
            if at.elapsed() < self.ttl {
                return processes.clone();
            }
        }
        let processes = self.source.unity_processes();
        *cache = Some((Instant::now(), processes.clone()));
        processes
    }
}

/// Case and separators ignored: `D:\Foo\Bar\` and `d:/foo/bar` compare equal.
pub fn normalize_path(path: &str) -> String {
    path.trim()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_lowercase()
}

/// The value of `-projectPath` in a command line, quoted or not.
pub fn project_path_of(command_line: &str) -> Option<String> {
    let lowered = command_line.to_lowercase();
    let start = lowered.find(PROJECT_PATH_FLAG)? + PROJECT_PATH_FLAG.len();
    let rest = command_line[start..].trim_start();
    if let Some(quoted) = rest.strip_prefix('"') {
        return quoted.split('"').next().map(str::to_string);
    }
    rest.split_whitespace().next().map(str::to_string)
}

/// The editor whose `-projectPath` is `project_path`.
pub fn find_editor<'a>(
    processes: &'a [UnityProcess],
    project_path: &str,
) -> Option<&'a UnityProcess> {
    let wanted = normalize_path(project_path);
    processes.iter().find(|process| {
        project_path_of(&process.command_line).is_some_and(|path| normalize_path(&path) == wanted)
    })
}

/// The most recently written readable JSON file of `dir` with its write time.
fn newest_json(dir: &Path) -> Option<(Value, SystemTime)> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| Some((entry.path(), entry.metadata().ok()?.modified().ok()?)))
        .collect();
    files.sort_by_key(|(_, modified)| std::cmp::Reverse(*modified));
    files.into_iter().find_map(|(path, modified)| {
        let text = aidp_core::text::read_text_lenient(path).ok()?;
        Some((serde_json::from_str(&text).ok()?, modified))
    })
}

fn results_dir(project: &Path, parts: &[&str; 3]) -> std::path::PathBuf {
    parts
        .iter()
        .fold(project.to_path_buf(), |dir, part| dir.join(part))
}

fn iso(time: SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(time).to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// The latest compile gate result (C1) as written, `null` when there is none.
pub fn last_compile(project: &Path) -> Value {
    newest_json(&results_dir(project, &COMPILE_RESULTS)).map_or(Value::Null, |(value, _)| value)
}

/// `{ id, passed, finishedAt }` of the latest playtest result (C2), `null` when there is none.
/// `finishedAt` is the result's own field, else the time the file was written.
pub fn last_playtest(project: &Path) -> Value {
    let Some((value, modified)) = newest_json(&results_dir(project, &PLAYTEST_RESULTS)) else {
        return Value::Null;
    };
    json!({
        "id": value.get("id").cloned().unwrap_or(Value::Null),
        "passed": value.get("passed").cloned().unwrap_or(Value::Null),
        "finishedAt": value
            .get("finishedAt")
            .and_then(Value::as_str)
            .map_or_else(|| iso(modified), str::to_string),
    })
}

/// The editor's last heartbeat (`.aidp/editor/heartbeat.json`, written by the playtest package every
/// two seconds) as written, `null` when there is none. A reader compares `at` with `staleAfterSeconds`.
pub fn last_heartbeat(project: &Path) -> Value {
    let path = results_dir(project, &HEARTBEAT_FILE);
    aidp_core::text::read_text_lenient(&path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null)
}

/// The body of `GET /api/unity/status` for the project at `project_path`.
pub fn status(
    project_id: &str,
    project_path: &str,
    processes: &[UnityProcess],
    holder: Option<&ResourceHolder>,
    waiting: &[String],
) -> Value {
    let editor = find_editor(processes, project_path);
    let project = Path::new(project_path);
    json!({
        "project": project_id,
        "editor": {
            "running": editor.is_some(),
            "pid": editor.map(|process| process.pid),
            "projectPath": project_path,
            "heartbeat": last_heartbeat(project),
        },
        "lock": {
            "held": holder.is_some(),
            "holder": holder,
            "waiting": waiting,
        },
        "lastCompile": last_compile(project),
        "lastPlaytest": last_playtest(project),
    })
}

#[cfg(test)]
#[path = "../tests/unit/unity_status_tests.rs"]
mod tests;
