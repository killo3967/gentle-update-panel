//! Which running processes can interfere with an update.
//!
//! `gentle-ai` itself force-stops engram on Windows before replacing the binary
//! (`Stop-Process -Force`), so engram is a *warning*, not a blocker. A live pi
//! session, on the other hand, would keep running against files that change
//! underneath it: that is a block.

use serde::{Deserialize, Serialize};
use sysinfo::{ProcessesToUpdate, System};

use crate::domain::ItemKind;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Blocker {
    pub name: String,
    pub pid: u32,
    /// `pi`, `engram`, `gentle-ai`, `gga` or `codegraph`.
    pub kind: String,
    pub command: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Impact {
    None,
    Warn,
    Block,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightStep {
    pub id: String,
    pub label: String,
    pub impact: Impact,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightReport {
    pub blockers: Vec<Blocker>,
    pub steps: Vec<PreflightStep>,
}

impl PreflightReport {
    pub fn has_interference(&self) -> bool {
        self.steps.iter().any(|step| step.impact != Impact::None)
    }
}

/// Classifies a process from its executable name and full command line.
/// Pure, so the heuristics are testable without spawning anything.
pub fn classify(name: &str, command: &str) -> Option<&'static str> {
    let name = name.to_ascii_lowercase();
    let command = command.to_ascii_lowercase();

    // The pi MCP wrapper for engram is a `node -e` process whose command line
    // mentions engram; treat it as engram, not as pi.
    if name.contains("engram") || (name.starts_with("node") && command.contains("engram")) {
        return Some("engram");
    }
    if name.contains("gentle-ai") {
        return Some("gentle-ai");
    }
    if name.contains("gga") {
        return Some("gga");
    }
    if name.contains("codegraph") || command.contains("codegraph") {
        return Some("codegraph");
    }
    if (name.starts_with("node") || name.starts_with("pi"))
        && (command.contains("pi-coding-agent")
            || command.contains(".pi\\agent")
            || command.contains(".pi/agent"))
    {
        return Some("pi");
    }
    None
}

/// Live snapshot of processes that could interfere. Never spawns a shell.
pub fn detect() -> Vec<Blocker> {
    let own = std::process::id();
    let mut system = System::new_all();
    system.refresh_processes(ProcessesToUpdate::All, true);

    let mut found: Vec<Blocker> = Vec::new();
    for (pid, process) in system.processes() {
        let pid_value = pid.as_u32();
        if pid_value == own {
            continue;
        }
        // Children of this process are our own CLI calls (doctor, upgrade…).
        if process.parent().map(|parent| parent.as_u32()) == Some(own) {
            continue;
        }

        let name = process.name().to_string_lossy().to_string();
        if name.eq_ignore_ascii_case("gentle-update-panel.exe") {
            continue;
        }
        let command = process
            .cmd()
            .iter()
            .map(|part| part.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");

        if let Some(kind) = classify(&name, &command) {
            found.push(Blocker {
                name: if name.is_empty() {
                    kind.to_string()
                } else {
                    name
                },
                pid: pid_value,
                kind: kind.to_string(),
                command: command.chars().take(240).collect(),
            });
        }
    }

    found.sort_by_key(|blocker| (blocker.kind.clone(), blocker.pid));
    found.dedup_by(|left, right| left.pid == right.pid);
    found
}

/// How an item is affected by the live processes.
pub fn impact(item_id: &str, kind: ItemKind, blockers: &[Blocker]) -> (Impact, Option<String>) {
    let running = |wanted: &str| blockers.iter().any(|blocker| blocker.kind == wanted);

    if matches!(kind, ItemKind::Pi | ItemKind::PiPackage) && running("pi") {
        return (
            Impact::Block,
            Some(
                "pi está en ejecución: pi o el paquete cambiarían bajo una sesión viva; reinicia pi después"
                    .to_string(),
            ),
        );
    }

    if item_id == "engram" && running("engram") {
        return (
            Impact::Warn,
            Some(
                "gentle-ai cerrará engram (Stop-Process -Force) antes de reemplazarlo; si tienes pi abierto, perderá memoria hasta reiniciar"
                    .to_string(),
            ),
        );
    }

    if item_id == "gentle-ai" && running("gentle-ai") {
        return (
            Impact::Warn,
            Some("se reemplaza el binario de gentle-ai mientras está en uso".to_string()),
        );
    }

    if item_id == "gga" && running("gga") {
        return (
            Impact::Warn,
            Some("se reemplaza gga mientras está en uso".to_string()),
        );
    }

    (Impact::None, None)
}

/// Appends an actionable hint when a failure looks like a file lock.
pub fn lock_hint(tool_name: &str, output: &str, blockers: &[Blocker]) -> Option<String> {
    let text = output.to_ascii_lowercase();
    let locked = [
        "being used by another process",
        "access is denied",
        "access denied",
        "acceso denegado",
        "otro proceso",
        "text file busy",
    ]
    .iter()
    .any(|needle| text.contains(needle));

    if !locked {
        return None;
    }

    let kind = if tool_name.contains("engram") {
        "engram"
    } else if tool_name.contains("gentle-ai") {
        "gentle-ai"
    } else if tool_name.contains("gga") {
        "gga"
    } else {
        "pi"
    };

    let holders: Vec<String> = blockers
        .iter()
        .filter(|blocker| blocker.kind == kind)
        .map(|blocker| format!("{} (PID {})", blocker.name, blocker.pid))
        .collect();

    if holders.is_empty() {
        Some(format!(
            "parece un fichero bloqueado por otro proceso; cierra {kind} y reintenta"
        ))
    } else {
        Some(format!(
            "parece un fichero bloqueado por {}; ciérralo y reintenta",
            holders.join(", ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocker(kind: &str) -> Blocker {
        Blocker {
            name: format!("{kind}.exe"),
            pid: 42,
            kind: kind.to_string(),
            command: String::new(),
        }
    }

    #[test]
    fn classifies_pi_sessions_and_its_helpers() {
        assert_eq!(
            classify(
                "node.exe",
                r#""C:\Program Files\nodejs\node.exe" C:\Users\t\.pi\agent\install\releases\1.4.0\node_modules\@earendil-works\pi-coding-agent\dist\index.js"#
            ),
            Some("pi")
        );
        assert_eq!(
            classify("node.exe", "node C:/x/node_modules/.pi/agent/bin/pi.js"),
            Some("pi")
        );
        assert_eq!(classify("node.exe", "node -e bundle"), None);
    }

    #[test]
    fn classifies_ecosystem_tools() {
        assert_eq!(classify("engram.exe", "engram serve"), Some("engram"));
        assert_eq!(
            classify("node.exe", "node -e \"spawn('engram')\""),
            Some("engram")
        );
        assert_eq!(classify("gentle-ai.exe", "gentle-ai doctor"), Some("gentle-ai"));
        assert_eq!(classify("gga.cmd", "gga --version"), Some("gga"));
        assert_eq!(
            classify("node.exe", "node .../codegraph/npm-shim.js serve --mcp"),
            Some("codegraph")
        );
        assert_eq!(classify("code.exe", "Code"), None);
        assert_eq!(classify("gentle-update-panel.exe", "app"), None);
    }

    #[test]
    fn pi_blocks_pi_and_packages_only() {
        let blockers = vec![blocker("pi")];
        assert_eq!(impact("pi", ItemKind::Pi, &blockers).0, Impact::Block);
        assert_eq!(
            impact("pkg:npm:gentle-pi", ItemKind::PiPackage, &blockers).0,
            Impact::Block
        );
        assert_eq!(impact("engram", ItemKind::Ecosystem, &blockers).0, Impact::None);
        assert_eq!(
            impact("gentle-ai", ItemKind::Ecosystem, &blockers).0,
            Impact::None
        );
    }

    #[test]
    fn engram_is_a_warning_because_gentle_ai_stops_it() {
        let blockers = vec![blocker("engram")];
        let (level, reason) = impact("engram", ItemKind::Ecosystem, &blockers);
        assert_eq!(level, Impact::Warn);
        assert!(reason.unwrap_or_default().contains("Stop-Process"));
    }

    #[test]
    fn lock_hint_names_the_holder() {
        let blockers = vec![blocker("engram")];
        let hint = lock_hint(
            "engram",
            "Access is denied: the file is being used by another process",
            &blockers,
        )
        .expect("hint");
        assert!(hint.contains("PID 42"));

        assert!(lock_hint("engram", "all good", &blockers).is_none());
    }

    /// Live check: prints what this machine is running.
    /// Run with: cargo test --lib live_detects -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_detects_pi_and_engram() {
        let found = detect();
        for blocker in &found {
            println!("{:>6}  {:<10}  {}", blocker.pid, blocker.kind, blocker.command);
        }
        assert!(found.iter().any(|blocker| blocker.kind == "pi"), "no pi detected");
        assert!(
            found.iter().any(|blocker| blocker.kind == "engram"),
            "no engram detected"
        );
    }

    #[test]
    fn report_flags_interference_only_when_a_step_is_affected() {
        let clean = PreflightReport {
            blockers: vec![blocker("codegraph")],
            steps: vec![PreflightStep {
                id: "engram".to_string(),
                label: "engram".to_string(),
                impact: Impact::None,
                reason: None,
            }],
        };
        assert!(!clean.has_interference());

        let warned = PreflightReport {
            blockers: vec![blocker("engram")],
            steps: vec![PreflightStep {
                id: "engram".to_string(),
                label: "engram".to_string(),
                impact: Impact::Warn,
                reason: Some("x".to_string()),
            }],
        };
        assert!(warned.has_interference());
    }
}
