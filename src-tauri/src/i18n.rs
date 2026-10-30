//! Text the panel shows outside the webview: tray labels, tooltips and
//! notifications. The webview has its own dictionaries under `src/i18n`; these
//! are the handful of strings that never pass through it.
//!
//! Every key carries all six languages in one row, so a missing translation is a
//! short array and not a silently absent branch.

use crate::locale::target_language;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Lang {
    Es,
    En,
    Pt,
    Fr,
    It,
    De,
}

impl Lang {
    fn from_tag(tag: &str) -> Lang {
        match tag.trim().to_ascii_lowercase().split(['-', '_']).next() {
            Some("en") => Lang::En,
            Some("pt") => Lang::Pt,
            Some("fr") => Lang::Fr,
            Some("it") => Lang::It,
            Some("de") => Lang::De,
            _ => Lang::Es,
        }
    }

    fn index(self) -> usize {
        match self {
            Lang::Es => 0,
            Lang::En => 1,
            Lang::Pt => 2,
            Lang::Fr => 3,
            Lang::It => 4,
            Lang::De => 5,
        }
    }
}

fn lang() -> Lang {
    Lang::from_tag(&target_language())
}

/// One row per key: [español, English, português, français, italiano, Deutsch].
const ROWS: &[(&str, [&str; 6])] = &[
    (
        "tray.open",
        [
            "Abrir panel",
            "Open panel",
            "Abrir painel",
            "Ouvrir le panneau",
            "Apri pannello",
            "Panel öffnen",
        ],
    ),
    (
        "tray.check",
        [
            "Comprobar ahora",
            "Check now",
            "Verificar agora",
            "Vérifier maintenant",
            "Controlla ora",
            "Jetzt prüfen",
        ],
    ),
    (
        "tray.doctor",
        [
            "Ejecutar doctor",
            "Run doctor",
            "Executar doctor",
            "Lancer le doctor",
            "Esegui doctor",
            "Doctor ausführen",
        ],
    ),
    (
        "tray.updatePanel",
        [
            "Actualizar el panel",
            "Update the panel",
            "Atualizar o painel",
            "Mettre à jour le panneau",
            "Aggiorna il pannello",
            "Panel aktualisieren",
        ],
    ),
    (
        "tray.updatePanelTo",
        [
            "Actualizar el panel a {version}",
            "Update the panel to {version}",
            "Atualizar o painel para {version}",
            "Mettre à jour le panneau vers {version}",
            "Aggiorna il pannello a {version}",
            "Panel auf {version} aktualisieren",
        ],
    ),
    (
        "tray.quit",
        ["Salir", "Quit", "Sair", "Quitter", "Esci", "Beenden"],
    ),
    (
        "tray.tooltip.idle",
        [
            "Gentle Update Panel · al día",
            "Gentle Update Panel · up to date",
            "Gentle Update Panel · em dia",
            "Gentle Update Panel · à jour",
            "Gentle Update Panel · aggiornato",
            "Gentle Update Panel · aktuell",
        ],
    ),
    (
        "tray.tooltip.pending",
        [
            "{count} pendiente(s)",
            "{count} pending",
            "{count} pendente(s)",
            "{count} en attente",
            "{count} in sospeso",
            "{count} ausstehend",
        ],
    ),
    (
        "tray.tooltip.panel",
        [
            "panel {version} disponible",
            "panel {version} available",
            "painel {version} disponível",
            "panneau {version} disponible",
            "pannello {version} disponibile",
            "Panel {version} verfügbar",
        ],
    ),
    (
        "notify.updates.one",
        [
            "1 actualización disponible",
            "1 update available",
            "1 atualização disponível",
            "1 mise à jour disponible",
            "1 aggiornamento disponibile",
            "1 Aktualisierung verfügbar",
        ],
    ),
    (
        "notify.updates.many",
        [
            "{count} actualizaciones disponibles",
            "{count} updates available",
            "{count} atualizações disponíveis",
            "{count} mises à jour disponibles",
            "{count} aggiornamenti disponibili",
            "{count} Aktualisierungen verfügbar",
        ],
    ),
    (
        "notify.panel.title",
        [
            "Panel {version} disponible",
            "Panel {version} available",
            "Painel {version} disponível",
            "Panneau {version} disponible",
            "Pannello {version} disponibile",
            "Panel {version} verfügbar",
        ],
    ),
    (
        "notify.panel.body",
        [
            "Se actualiza desde la bandeja o el panel.",
            "Update it from the tray or the panel.",
            "Atualiza-se a partir da bandeja ou do painel.",
            "Mise à jour depuis la barre d'état ou le panneau.",
            "Si aggiorna dalla barra o dal pannello.",
            "Aktualisierung über Taskleiste oder Panel.",
        ],
    ),
];

/// Translation of `key` in the system language, with Spanish as the fallback.
pub fn t(key: &str) -> &'static str {
    let index = lang().index();
    ROWS.iter()
        .find(|(candidate, _)| *candidate == key)
        .map(|(_, translations)| translations[index])
        // A missing key shows itself: visible in the tray, easy to spot.
        .unwrap_or_else(|| Box::leak(key.to_string().into_boxed_str()))
}

/// Same, filling `{name}` placeholders.
pub fn t_args(key: &str, args: &[(&str, &str)]) -> String {
    let mut text = t(key).to_string();
    for (name, value) in args {
        text = text.replace(&format!("{{{name}}}"), value);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_has_every_row() {
        for (key, translations) in ROWS {
            assert_eq!(translations.len(), 6, "fila incompleta: {key}");
            for text in translations {
                assert!(!text.trim().is_empty(), "traducción vacía en {key}");
            }
        }
    }

    #[test]
    fn rows_do_not_repeat_keys() {
        let mut keys: Vec<&str> = ROWS.iter().map(|(key, _)| *key).collect();
        let total = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), total, "hay claves repetidas en la tabla");
    }

    #[test]
    fn placeholders_match_across_languages() {
        for (key, translations) in ROWS {
            let spanish = translations[0].matches('{').count();
            for text in translations {
                assert_eq!(
                    text.matches('{').count(),
                    spanish,
                    "marcadores distintos en {key}: {text}"
                );
            }
        }
    }

    #[test]
    fn reads_the_language_from_the_tag() {
        assert_eq!(Lang::from_tag("es-ES"), Lang::Es);
        assert_eq!(Lang::from_tag("en-GB"), Lang::En);
        assert_eq!(Lang::from_tag("pt_BR"), Lang::Pt);
        assert_eq!(Lang::from_tag("fr"), Lang::Fr);
        assert_eq!(Lang::from_tag("it-IT"), Lang::It);
        assert_eq!(Lang::from_tag("de-DE"), Lang::De);
        assert_eq!(Lang::from_tag("ru-RU"), Lang::Es);
        assert_eq!(Lang::from_tag(""), Lang::Es);
    }

    #[test]
    fn fills_placeholders() {
        let text = t_args("tray.updatePanelTo", &[("version", "0.7.0")]);
        assert!(text.contains("0.7.0"), "{text}");
        assert!(!text.contains('{'), "{text}");
    }

    /// The journal keys are written in Rust and rendered by the webview: a typo
    /// here shows the raw key in the Registro, so the two sides are checked.
    #[test]
    fn journal_keys_exist_in_the_webview_dictionary() {
        let dictionary = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../src/i18n/es.ts"
        ))
        .expect("diccionario español");

        let pattern = regex::Regex::new(r#"\"(journal\.[a-zA-Z.]+)\""#).expect("regex");
        let mut checked = 0;

        for file in ["service.rs", "upgrade.rs", "commands.rs", "panel.rs", "lib.rs"] {
            let source = std::fs::read_to_string(format!(
                "{}/src/{file}",
                env!("CARGO_MANIFEST_DIR")
            ))
            .expect("fuente");

            for capture in pattern.captures_iter(&source) {
                let key = &capture[1];
                checked += 1;
                assert!(
                    dictionary.contains(&format!("\"{key}\":")),
                    "falta la clave {key} en src/i18n/es.ts"
                );
            }
        }

        assert!(checked > 10, "solo se revisaron {checked} claves");
    }
}
