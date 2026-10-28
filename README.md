# Gentle Update Panel

Panel de escritorio para Windows que vigila las actualizaciones del ecosistema Gentle AI y de pi, muestra qué trae cada versión y aplica solo lo que tú elijas. Vive en la bandeja del sistema y comprueba en segundo plano cada `N` minutos.

```
gentle-ai · engram · gga        <- gentle-ai update
pi                              <- pi --version + npm
paquetes de pi (npm y git)      <- pi list + package.json + npm registry
```

## Qué hace

- **Comprueba** el ecosistema (`gentle-ai update`), la versión de `pi` y cada paquete declarado en `pi list`.
- **Muestra el "qué trae de nuevo"**: una versión a la vez, con flechas `←` `→` para recorrer el historial (hasta 8 releases), enlaces clicables que abren el navegador (incluidos los autoenlaces de GitHub a `#123` y a commits) y menú contextual propio sobre cada enlace.
- **Busca en las notas**: un cuadro de búsqueda recorre todas las releases ya descargadas, de cualquier pieza, y salta a la versión concreta.
- **Comprueba qué hay corriendo antes de actualizar**: detecta pi, engram, gentle-ai, gga y el daemon de CodeGraph sin lanzar PowerShell. Si algo interfiere, abre un diálogo con los procesos, marca en rojo los pasos bloqueados (y no los preselecciona) y deja cancelar. `pi` bloquea sus propias actualizaciones y las de sus paquetes; `engram` solo avisa, porque `gentle-ai` lo cierra con `Stop-Process -Force` antes de reemplazarlo.
- **Actualiza por selección**: `gentle-ai upgrade <tool>`, `pi update --self` o `pi update --extension <source>`, con la salida en vivo en un cajón de actividad.
- **Traduce las notas**: con dos endpoints gratuitos y sin clave (Google `translate_a/single` y Microsoft `translate_edge/translatetext`). Protege código, rutas y enlaces con marcadores antes de enviar, cachea por versión y cae al segundo motor si el primero falla. Conmutador *Traducir / Ver original* y aviso de "traducido automáticamente".
- **Ignora piezas**: en Ajustes puedes silenciar gentle-ai, engram, gga, pi o cualquier paquete. Siguen listadas (para poder revertirlo), pero no cuentan como pendientes, no avisan, no se actualizan y ni siquiera se consulta su versión publicada.
- **Serializa el trabajo**: una comprobación y una actualización nunca corren a la vez (`work_lock`), y el scheduler espera si hay un apply en curso.
- **Verifica**: ejecuta `gentle-ai doctor` después de aplicar (configurable) y muestra checks, remedios y salida completa.
- **Silencia checks del doctor**: en Ajustes → *Reglas del doctor* puedes ignorar comprobaciones concretas por id (`tool:hermes` para siempre) y activar el silencio automático de `tool:*` cuando esa herramienta no está instalada (por ejemplo `gga` deshabilitado). Las silenciadas no cuentan para el estado, pero siguen consultables en un desplegable.
- **Verifica la instalación de pi**: como pi no tiene `doctor`, el panel comprueba por su cuenta que cada paquete existe en disco, que su `package.json` se lee, que declara versión y que los recursos que anuncia (`pi.extensions`, `pi.skills`, `pi.themes`, `pi.prompts`) existen. Además valida `settings.json` (duplicados) y el árbol npm de extensiones.
- **Vive en la bandeja**: badge con pendientes, notificación toast, menú (Abrir, Comprobar ahora, Doctor, Salir) y arranque con Windows (HKCU, sin admin).

## Uso

```bash
npm install
npm run tauri dev      # desarrollo con HMR
npm run tauri build    # instalador NSIS en src-tauri/target/release/bundle/nsis/
```

- **Cerrar** (X) oculta la ventana; se sale desde el menú de la bandeja.
- **Ajustes**: frecuencia (15 min → 24 h, o Manual), automatismo (Avisar / Confirmar / Automático), notificaciones, doctor automático, arranque minimizado, arranque con Windows y la lista de **piezas ignoradas**.
- **Arranque con Windows** usa la clave `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` con el argumento `--minimized`.

## Arquitectura

```
src/                 UI (TypeScript + Vite, sin frameworks)
  store.ts           estado + acciones, eventos de Tauri
  ui/format.ts       escaping, markdown seguro, iconos, gauge SVG
  ui/views.ts        rail, resumen, detalle, doctor, ajustes
  ui/shell.ts        titlebar, delegación de eventos, drawer, toasts
src-tauri/src/
  parsers.rs         parsers puros de salidas CLI + tests
  cli.rs             ejecución de CLIs (shims .cmd, timeouts, streaming)
  probe.rs           sondas: gentle-ai, pi, paquetes, registro npm
  notes.rs           notas de versión con caché y reintento
  doctor.rs          gentle-ai doctor
  upgrade.rs         plan y aplicación de actualizaciones
  service.rs         ciclo de comprobación + notificaciones
  tray.rs            bandeja y visibilidad de la ventana
  scheduler.rs       comprobación periódica
  state.rs           config, snapshot y doctor persistidos
```

Decisiones que conviene conocer:

- `gentle-ai` **no** ofrece salida JSON para `update`/`doctor`: los parsers trabajan sobre texto real, con fixtures tomados de la máquina y cubiertos por tests.
- `pi update` sin banderas **solo actualiza pi**; los paquetes necesitan `--extensions`, `--all` o `--extension <source>`.
- En Windows, `pi` es un shim `.cmd` y en `PATH` conviven un script POSIX sin extensión y el `.cmd` real: el resolvedor solo acepta candidatos `PATHEXT`.
- `cmd /C` no entiende el escapado de Rust, así que la línea se pasa con `raw_arg`.
- El plugin `window-state` guarda solo tamaño, posición y maximizado: la visibilidad la decide la bandeja.
- `reqwest` usa **native-tls** (SChannel en Windows) y no rustls: el endpoint gratuito de Google devuelve 429 a la huella TLS de rustls y 200 a la de SChannel. Microsoft funciona con cualquiera.
- La traducción protege código y enlaces con `[[Tn]]`; si el traductor pierde un marcador, se descarta la traducción y se muestra el original.

## Datos

Todo se guarda en `%APPDATA%\dev.gentle.update-panel\`:

| Fichero | Contenido |
|---|---|
| `config.json` | frecuencia, automatismo, vista y pieza recordadas, lista de ignoradas |
| `snapshot.json` | última comprobación (se muestra al instante al abrir) |
| `doctor.json` | último informe del doctor |
| `pi-health.json` | última verificación de instalación de pi y sus paquetes |
| `notes-cache.json` | notas de versión (TTL 6 h) |
| `translations.json` | traducciones cacheadas por versión, motor e idioma |

## Desarrollo

```bash
cd src-tauri && cargo test          # 22 tests de parsers, contadores y planificación
cargo test --lib -- --ignored       # comprobaciones en vivo contra tus CLIs
```

`tools/` contiene utilidades usadas para generar iconos y capturar la ventana en pruebas visuales.
