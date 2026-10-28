//! Installation health for pi and each of its packages.
//!
//! `pi` has no doctor, so this module answers the questions that actually break
//! an install: does the declared path exist, is `package.json` readable, does it
//! declare a version, and do the resources it advertises exist on disk.

use std::path::Path;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::cli;
use crate::parsers::PackageEntry;

const PI_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const NPM_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

const RESOURCE_KEYS: [&str; 5] = ["extensions", "skills", "themes", "prompts", "promptTemplates"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthCheck {
    /// Short label: `ruta`, `package.json`, `version`, `recursos`…
    pub id: String,
    /// `ok` | `warn` | `fail`
    pub marker: String,
    pub message: String,
    pub remedy: Option<String>,
}

impl HealthCheck {
    fn ok(id: &str, message: impl Into<String>) -> Self {
        Self {
            id: id.to_string(),
            marker: "ok".to_string(),
            message: message.into(),
            remedy: None,
        }
    }

    fn warn(id: &str, message: impl Into<String>, remedy: impl Into<String>) -> Self {
        Self {
            id: id.to_string(),
            marker: "warn".to_string(),
            message: message.into(),
            remedy: Some(remedy.into()),
        }
    }

    fn fail(id: &str, message: impl Into<String>, remedy: impl Into<String>) -> Self {
        Self {
            id: id.to_string(),
            marker: "fail".to_string(),
            message: message.into(),
            remedy: Some(remedy.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageHealth {
    pub id: String,
    pub name: String,
    pub source: String,
    pub path: String,
    pub checks: Vec<HealthCheck>,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiHealth {
    pub checked_at: String,
    pub status: String,
    pub version: Option<String>,
    /// Global checks (settings, npm tree, CLI reachable).
    pub checks: Vec<HealthCheck>,
    pub packages: Vec<PackageHealth>,
}

impl PiHealth {
    pub fn failing(&self) -> usize {
        self.packages
            .iter()
            .filter(|package| package.status == "fail")
            .count()
            + usize::from(self.status == "fail")
    }
}

/// Worst status wins: fail > warn > ok.
pub fn rollup(checks: &[HealthCheck]) -> String {
    if checks.iter().any(|check| check.marker == "fail") {
        "fail".to_string()
    } else if checks.iter().any(|check| check.marker == "warn") {
        "warn".to_string()
    } else {
        "ok".to_string()
    }
}

/// Verifies one installed package on disk. Never touches the network.
pub fn verify_package(entry: &PackageEntry) -> PackageHealth {
    let name = crate::probe::display_name(&entry.source);
    let path = std::path::PathBuf::from(&entry.path);
    let mut checks: Vec<HealthCheck> = Vec::new();

    if !path.is_dir() {
        checks.push(HealthCheck::fail(
            "ruta",
            format!("no existe {}", path.display()),
            format!("pi install {}", entry.source),
        ));
        return PackageHealth {
            id: format!("pkg:{}", entry.source),
            name,
            source: entry.source.clone(),
            path: entry.path.clone(),
            status: rollup(&checks),
            checks,
        };
    }
    checks.push(HealthCheck::ok(
        "ruta",
        format!("instalado en {}", path.display()),
    ));

    let manifest_path = path.join("package.json");
    let manifest = match std::fs::read_to_string(&manifest_path) {
        Ok(raw) => match serde_json::from_str::<serde_json::Value>(&raw) {
            Ok(value) => {
                checks.push(HealthCheck::ok("package.json", "manifiesto legible"));
                Some(value)
            }
            Err(error) => {
                checks.push(HealthCheck::fail(
                    "package.json",
                    format!("JSON inválido: {error}"),
                    format!("pi install {}", entry.source),
                ));
                None
            }
        },
        Err(error) => {
            checks.push(HealthCheck::fail(
                "package.json",
                format!("no se pudo leer {}: {error}", manifest_path.display()),
                format!("pi install {}", entry.source),
            ));
            None
        }
    };

    if let Some(manifest) = manifest {
        match manifest.get("version").and_then(|value| value.as_str()) {
            Some(version) if !version.trim().is_empty() => checks.push(HealthCheck::ok(
                "version",
                format!("versión {version}"),
            )),
            _ => checks.push(HealthCheck::warn(
                "version",
                "el manifiesto no declara versión",
                format!("reinstala el paquete: pi install {}", entry.source),
            )),
        }

        let declared = declared_resources(&manifest);
        if declared.is_empty() {
            checks.push(HealthCheck::ok("recursos", "sin recursos declarados"));
        } else {
            let missing: Vec<String> = declared
                .iter()
                .filter(|resource| !resource_exists(&path, resource))
                .cloned()
                .collect();
            if missing.is_empty() {
                checks.push(HealthCheck::ok(
                    "recursos",
                    format!("{} recurso(s) presentes", declared.len()),
                ));
            } else {
                checks.push(HealthCheck::warn(
                    "recursos",
                    format!("faltan rutas declaradas: {}", missing.join(", ")),
                    format!("reinstala el paquete: pi install {}", entry.source),
                ));
            }
        }
    }

    PackageHealth {
        id: format!("pkg:{}", entry.source),
        name,
        source: entry.source.clone(),
        path: entry.path.clone(),
        status: rollup(&checks),
        checks,
    }
}

/// Sources declared more than once in `settings.json`.
pub fn duplicate_sources(settings: &serde_json::Value) -> Vec<String> {
    let Some(list) = settings.get("packages").and_then(|value| value.as_array()) else {
        return Vec::new();
    };

    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for item in list {
        let source = match item {
            serde_json::Value::String(value) => Some(value.clone()),
            serde_json::Value::Object(map) => map
                .get("source")
                .and_then(|value| value.as_str())
                .map(str::to_string),
            _ => None,
        };
        if let Some(source) = source {
            *counts.entry(source).or_default() += 1;
        }
    }

    let mut duplicates: Vec<String> = counts
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .map(|(source, _)| source)
        .collect();
    duplicates.sort();
    duplicates
}

/// Runs the local checks: pi CLI, `pi list`, settings.json and the npm tree.
pub async fn run() -> PiHealth {
    let mut checks: Vec<HealthCheck> = Vec::new();
    let mut packages: Vec<PackageHealth> = Vec::new();
    let mut version = None;

    match cli::run("pi", &["--version".to_string()], PI_TIMEOUT).await {
        Ok(output) if !output.timed_out && !output.stdout.trim().is_empty() => {
            let value = output.stdout.trim().trim_start_matches('v').to_string();
            checks.push(HealthCheck::ok("pi:cli", format!("pi {value} responde")));
            version = Some(value);
        }
        Ok(output) => checks.push(HealthCheck::fail(
            "pi:cli",
            format!("`pi --version` no devolvió versión: {}", output.combined()),
            "reinstala pi o revisa el PATH",
        )),
        Err(error) => checks.push(HealthCheck::fail(
            "pi:cli",
            format!("pi no se pudo ejecutar: {error}"),
            "reinstala pi o revisa el PATH",
        )),
    }

    match cli::run("pi", &["list".to_string()], PI_TIMEOUT).await {
        Ok(output) if !output.timed_out => {
            let entries = crate::parsers::parse_pi_list(&output.stdout);
            if entries.is_empty() {
                checks.push(HealthCheck::warn(
                    "pi:paquetes",
                    "`pi list` no devolvió paquetes",
                    "comprueba que ~/.pi/agent/settings.json declara paquetes",
                ));
            } else {
                checks.push(HealthCheck::ok(
                    "pi:paquetes",
                    format!("{} paquete(s) declarados", entries.len()),
                ));
                packages = entries.iter().map(verify_package).collect();
            }
        }
        Ok(_) => checks.push(HealthCheck::fail(
            "pi:paquetes",
            "`pi list` no respondió a tiempo",
            "ejecuta `pi list` en una terminal para ver el error",
        )),
        Err(error) => checks.push(HealthCheck::fail(
            "pi:paquetes",
            format!("`pi list` falló: {error}"),
            "reinstala pi o revisa el PATH",
        )),
    }

    if let Some(settings_path) = settings_path() {
        match std::fs::read_to_string(&settings_path) {
            Ok(raw) => match serde_json::from_str::<serde_json::Value>(&raw) {
                Ok(settings) => {
                    let duplicates = duplicate_sources(&settings);
                    if duplicates.is_empty() {
                        checks.push(HealthCheck::ok(
                            "settings.json",
                            format!("JSON válido en {}", settings_path.display()),
                        ));
                    } else {
                        checks.push(HealthCheck::warn(
                            "settings.json",
                            format!(
                                "declaraciones duplicadas: {}",
                                duplicates.join(", ")
                            ),
                            "deja solo una entrada por paquete en settings.json",
                        ));
                    }
                }
                Err(error) => checks.push(HealthCheck::fail(
                    "settings.json",
                    format!("JSON inválido: {error}"),
                    "repara o restaura settings.json",
                )),
            },
            Err(error) => checks.push(HealthCheck::warn(
                "settings.json",
                format!("no se pudo leer {}: {error}", settings_path.display()),
                "comprueba permisos del fichero",
            )),
        }
    }

    if let Some(home) = cli::home_dir() {
        let npm_dir = home.join(".pi").join("agent").join("npm");
        if npm_dir.is_dir() {
            let args = vec![
                "ls".to_string(),
                "--depth=0".to_string(),
                "--omit=dev".to_string(),
            ];
            match cli::run_in("npm", &args, NPM_TIMEOUT, &npm_dir).await {
                Ok(output) if output.ok() => checks.push(HealthCheck::ok(
                    "npm:tree",
                    "todas las dependencias de extensiones resuelven",
                )),
                Ok(output) => {
                    let detail = first_lines(&output.combined(), 3);
                    checks.push(HealthCheck::fail(
                        "npm:tree",
                        format!("npm reporta problemas: {detail}"),
                        "ejecuta `npm install` en ~/.pi/agent/npm",
                    ));
                }
                Err(error) => checks.push(HealthCheck::warn(
                    "npm:tree",
                    format!("no se pudo comprobar el árbol npm: {error}"),
                    "ejecuta `npm ls --depth=0` en ~/.pi/agent/npm",
                )),
            }
        }
    }

    let status = rollup(&checks);

    PiHealth {
        checked_at: Utc::now().to_rfc3339(),
        status,
        version,
        checks,
        packages,
    }
}

pub fn settings_path() -> Option<std::path::PathBuf> {
    let home = cli::home_dir()?;
    let path = home.join(".pi").join("agent").join("settings.json");
    path.is_file().then_some(path)
}

fn first_lines(text: &str, count: usize) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(count)
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Declared resource paths inside a package.json `pi` object.
fn declared_resources(package: &serde_json::Value) -> Vec<String> {
    let Some(pi) = package.get("pi").and_then(|value| value.as_object()) else {
        return Vec::new();
    };

    let mut resources = Vec::new();
    for key in RESOURCE_KEYS {
        let Some(value) = pi.get(key) else { continue };
        match value {
            serde_json::Value::String(path) => resources.push(path.clone()),
            serde_json::Value::Array(items) => {
                for item in items {
                    if let Some(path) = item.as_str() {
                        resources.push(path.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    resources
}

fn resource_exists(base: &Path, resource: &str) -> bool {
    // Entries can be plain paths or carry an enable/disable prefix.
    let trimmed = resource.trim_start_matches(['+', '-', '!']);
    let candidate = base.join(trimmed);
    candidate.exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("gup-health-{}-{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn entry(path: &Path, source: &str) -> PackageEntry {
        PackageEntry {
            source: source.to_string(),
            path: path.display().to_string(),
        }
    }

    #[test]
    fn healthy_package_passes_every_check() {
        let dir = scratch("ok");
        fs::create_dir_all(dir.join("extensions")).unwrap();
        fs::write(
            dir.join("package.json"),
            r#"{"name":"demo","version":"1.2.3","pi":{"extensions":["./extensions"]}}"#,
        )
        .unwrap();

        let health = verify_package(&entry(&dir, "npm:demo"));

        assert_eq!(health.status, "ok");
        assert_eq!(health.name, "demo");
        assert!(health
            .checks
            .iter()
            .all(|check| check.marker == "ok"), "checks: {:#?}", health.checks);
        assert!(health
            .checks
            .iter()
            .any(|check| check.id == "recursos"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_path_fails_and_stops_early() {
        let dir = scratch("missing");
        let gone = dir.join("node_modules").join("ghost");
        let health = verify_package(&entry(&gone, "npm:ghost"));

        assert_eq!(health.status, "fail");
        assert_eq!(health.checks[0].id, "ruta");
        assert_eq!(health.checks[0].marker, "fail");
        assert!(health.checks[0]
            .remedy
            .as_deref()
            .unwrap_or_default()
            .contains("pi install"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn broken_package_json_fails() {
        let dir = scratch("bad-json");
        fs::write(dir.join("package.json"), "{ not json").unwrap();

        let health = verify_package(&entry(&dir, "npm:broken"));

        assert_eq!(health.status, "fail");
        assert!(health
            .checks
            .iter()
            .any(|check| check.id == "package.json" && check.marker == "fail"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_declared_resource_warns() {
        let dir = scratch("resource");
        fs::write(
            dir.join("package.json"),
            r#"{"name":"demo","version":"1.0.0","pi":{"extensions":["./extensions"],"themes":["./themes"]}}"#,
        )
        .unwrap();
        fs::create_dir_all(dir.join("themes")).unwrap();

        let health = verify_package(&entry(&dir, "npm:demo"));

        assert_eq!(health.status, "warn");
        let resources = health
            .checks
            .iter()
            .find(|check| check.id == "recursos")
            .expect("recursos check");
        assert_eq!(resources.marker, "warn");
        assert!(resources.message.contains("extensions"));
        assert!(!resources.message.contains("themes"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_version_warns() {
        let dir = scratch("no-version");
        fs::write(dir.join("package.json"), r#"{"name":"demo"}"#).unwrap();

        let health = verify_package(&entry(&dir, "npm:demo"));

        assert_eq!(health.status, "warn");
        assert!(health
            .checks
            .iter()
            .any(|check| check.id == "version" && check.marker == "warn"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn detects_duplicate_sources_in_settings() {
        let settings = serde_json::json!({
            "packages": [
                "npm:gentle-engram",
                {"source": "npm:@scope/pkg", "extensions": ["+dist/index.js"]},
                "npm:gentle-engram",
                "npm:@scope/pkg",
                "git:github.com/o/r"
            ]
        });

        assert_eq!(
            duplicate_sources(&settings),
            vec!["npm:@scope/pkg".to_string(), "npm:gentle-engram".to_string()]
        );
    }

    #[test]
    fn duplicate_detection_tolerates_missing_packages() {
        assert!(duplicate_sources(&serde_json::json!({})).is_empty());
        assert!(duplicate_sources(&serde_json::json!({"packages": []})).is_empty());
    }

    #[test]
    fn rollup_takes_the_worst_marker() {
        let ok = vec![HealthCheck::ok("a", "b")];
        let warn = vec![HealthCheck::ok("a", "b"), HealthCheck::warn("c", "d", "e")];
        let fail = vec![
            HealthCheck::ok("a", "b"),
            HealthCheck::warn("c", "d", "e"),
            HealthCheck::fail("f", "g", "h"),
        ];
        assert_eq!(rollup(&ok), "ok");
        assert_eq!(rollup(&warn), "warn");
        assert_eq!(rollup(&fail), "fail");
    }
}
