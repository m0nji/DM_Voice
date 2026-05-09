# DM Voice

Push-to-Talk Speech-to-Text App für macOS. Hotkey halten → sprechen → loslassen → Text wird in das gerade aktive Textfeld eingefügt. Lokal, offline, mit Whisper.

## Features

- **Push-to-Talk** über frei wählbaren Hotkey (Standard: `Alt+Space`)
- **Whisper-Modelle** zur Auswahl (tiny / small / medium / large-v3-turbo / large-v3) — direkt im Tray-Menü umschaltbar
- **Lokal & offline** — kein Cloud-Upload, läuft mit Metal-Beschleunigung auf Apple Silicon
- **Text-Injection** in jede aktive App via Cmd+V (CGEvent + AX-API)
- **Auto-Start beim Login** als Option im Tray-Menü
- **Aufnahmelimit**: 60 Sekunden pro Push-to-Talk

## Installation

1. **Download**: aktuelles DMG aus den [Releases](https://github.com/m0nji/DM_Voice/releases) holen
2. **DMG öffnen** und `DM Voice.app` nach `/Applications` ziehen
3. **Erster Start**: Rechtsklick auf die App → "Öffnen" → im Dialog "Öffnen" bestätigen
   *(Die App ist nicht von Apple notarisiert — daher zeigt macOS einmalig die Gatekeeper-Warnung. Doppelklick funktioniert ab dem zweiten Mal.)*
4. **Permissions** beim ersten Start erteilen:
   - **Mikrofon** — für die Aufnahme
   - **Bedienungshilfen** — für das Einfügen des Textes per Cmd+V

Beim ersten Start wird automatisch das `large-v3-turbo`-Modell (~874 MB) heruntergeladen. Andere Modelle können danach im Tray-Menü angefordert werden.

## Benutzung

- **Aufnahme**: Hotkey halten (Standard `Alt+Space`), sprechen, loslassen → Text wird ins aktive Feld eingefügt
- **Tray-Icon klicken**: Menü mit Modell-Auswahl, Auto-Start-Toggle, Einstellungen, Beenden
- **Einstellungen** → Hotkey ändern, Permissions prüfen, Modelle herunter-/löschen

## Build aus dem Quellcode

Voraussetzungen: macOS 13+, Rust toolchain, Tauri CLI (`cargo install tauri-cli`).

```bash
git clone https://github.com/m0nji/DM_Voice.git
cd DM_Voice/src-tauri
cargo tauri build --bundles app,dmg
```

Das fertige Bundle liegt unter `src-tauri/target/release/bundle/macos/DM Voice.app`,
das DMG unter `src-tauri/target/release/bundle/dmg/`.

**Wichtig**: Wenn du selbst signierst, lass das Hardened-Runtime-Flag weg
(`codesign` ohne `--options runtime`) — sonst unterdrückt macOS den
Mikrofon-TCC-Prompt stillschweigend bei lokalen Self-Signed Certs.

## Lizenz

Privatprojekt — alle Rechte vorbehalten. © DM Apps
