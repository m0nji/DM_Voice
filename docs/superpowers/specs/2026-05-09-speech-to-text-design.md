# DM Voice — Speech-to-Text App Design

**Date:** 2026-05-09
**Status:** Approved

## Overview

DM Voice ist eine macOS-First Desktop-App, die per globalem Shortcut (Push-to-Talk) gesprochenen Text in jedes fokussierte Textfeld einfügt. Die Transkription läuft vollständig lokal via whisper.cpp mit Metal-Beschleunigung (Apple Silicon). Eine schwarze Pill-Overlay-Animation gibt visuelles Feedback während der Aufnahme. Später soll die App auch auf Windows laufen.

## Tech Stack

- **Framework:** Rust + Tauri 2
- **STT Engine:** whisper-rs (whisper.cpp Bindings) mit Metal-Backend
- **Audio Capture:** cpal
- **Text Injection:** CGEventPost (macOS Accessibility API) / SendInput (Windows, später)
- **Global Shortcuts:** tauri-plugin-global-shortcut
- **System Tray:** tauri-plugin-system-tray
- **Config:** TOML via `dirs` crate
- **Overlay/UI:** Tauri WebView (HTML/CSS/JS)
- **macOS Build Target:** `aarch64-apple-darwin` (Apple Silicon only, M4+)
- **Build Flags:** `WHISPER_METAL=1`, `GGML_METAL=1`
- **Distribution:** Signiertes `.dmg` (macOS), `.msi` (Windows, später)

## Architektur

Sieben Komponenten mit klaren Grenzen:

### 1. Audio Engine (`cpal`)
- Öffnet Mikrofon-Stream bei Shortcut-KeyDown
- Sammelt PCM-Audio in einem In-Memory-Buffer (16kHz, 16-bit mono — Whisper-Format)
- Berechnet RMS-Amplitude pro Chunk (~50ms) für Waveform-Animation
- Stoppt und liefert finalen Buffer bei Shortcut-KeyUp

### 2. STT Engine (`whisper-rs`)
- whisper.cpp kompiliert mit `WHISPER_METAL=1` für M4-GPU-Beschleunigung
- Standard-Modell: `ggml-large-v3-turbo-q5_0` (~874 MB Disk, ~900 MB RAM)
- Modell-Dateien in `~/Library/Application Support/DM-Voice/models/`
- Gibt transkribierten Text als String zurück

### 3. Text Injector
- macOS: Text wird intern in die Zwischenablage gelegt, dann `⌘V` via `CGEventPost` gesendet, danach wird der vorherige Clipboard-Inhalt wiederhergestellt
- Prüft ob ein Textfeld fokussiert ist bevor Injection
- Fallback (kein Textfeld): Text in Zwischenablage + macOS-Notification "Kein Textfeld aktiv — Text kopiert"
- Windows (später): `SendInput` Win32 API, selbe Clipboard-Paste-Strategie

### 4. Shortcut Manager (`tauri-plugin-global-shortcut`)
- Registriert globalen KeyDown/KeyUp-Hook im System
- Standard-Shortcut: `⌥Space` (Option+Space)
- KeyDown → Aufnahme starten
- KeyUp → Aufnahme stoppen, Transkription + Injection auslösen
- Shortcut konfigurierbar über Settings

### 5. Overlay Window
- Transparentes, always-on-top Tauri-WebView-Fenster
- Größe: 120×40px, zentriert auf dem Hauptbildschirm
- Keine Titelleiste, kein Schatten, nicht interaktiv (click-through)
- Empfängt `amplitude`-Events vom Rust-Core via Tauri-Event-Bus

**Visuelle Zustände:**

| Zustand | Beschreibung |
|---|---|
| Aufnahme | 5 weiße Balken, Höhe 4–24px, reagiert auf RMS-Amplitude, smooth interpoliert |
| Verarbeitung | Alle 5 Balken pulsen gleichmäßig langsam (Whisper rechnet) |
| Fertig | 200ms grüner Tint (`#22c55e`) → fade-out über 150ms |

**Farben:** Hintergrund `#000000`, Balken `#FFFFFF`. Exakt SuperWhisper-Mini-Stil.

### 6. System Tray
- Kleines Waveform-Icon in der macOS-Menüleiste
- Klick auf Icon → öffnet Settings-Fenster
- Rechtsklick → Kontextmenü mit "Beenden"
- Icon pulsiert während aktiver Aufnahme

### 7. Settings Window
- Kleines natives Tauri-Fenster (~300×200px)
- Öffnet sich bei Klick auf Tray-Icon
- **Shortcut-Recorder:** Klick → nächste Tastenkombination erfassen
- **Modell-Auswahl:** Dropdown mit Downloadstatus pro Modell
- Einstellungen gespeichert in:
  - macOS: `~/Library/Application Support/DM-Voice/config.toml`
  - Windows: `%APPDATA%\DM-Voice\config.toml`

## Datenfluss

```
[Shortcut KeyDown]
    → Audio Engine: Mikrofon öffnen, PCM-Stream starten
    → Overlay: fade-in, Idle-Animation startet
    → Tray-Icon: pulsiert

[Jeder Audio-Chunk ~50ms]
    → RMS-Amplitude berechnen
    → Tauri-Event "amplitude" → Overlay animiert Balken reaktiv

[Shortcut KeyUp]
    → Audio Engine: Stream stoppen, Buffer finalisieren
    → Overlay: wechselt zu Verarbeitungs-Animation
    → STT Engine: Audio-Buffer → Whisper Metal-Inferenz

[Transkription fertig]
    → Text-Injector: fokussiertes Textfeld vorhanden?
        JA  → CGEventPost: Text zeichenweise injizieren
        NEIN → Clipboard + Notification
    → Overlay: grüner Flash → fade-out
    → Tray-Icon: zurück zu normal
```

## Randfall-Behandlung

| Fall | Verhalten |
|---|---|
| Aufnahme < 300ms | Still verworfen, kein Whisper-Aufruf |
| Aufnahme > 30s | Automatisch stoppen und transkribieren |
| Whisper-Fehler | Notification "Transkription fehlgeschlagen", Overlay verschwindet |
| Kein Mikrofon | Notification "Kein Mikrofon gefunden" |
| Modell nicht heruntergeladen (Shortcut gedrückt) | Overlay zeigt Download-Fortschritt, Aufnahme startet erst danach |

## Modell-Management

Modelle werden on-demand aus Hugging Face heruntergeladen und lokal gecacht.

| Modell | Disk | RAM | Qualität Deutsch |
|---|---|---|---|
| `tiny` | 75 MB | ~150 MB | ausreichend |
| `small` | 244 MB | ~350 MB | gut |
| `medium` | 769 MB | ~700 MB | sehr gut |
| `large-v3-turbo` ★ | 874 MB | ~900 MB | exzellent |
| `large-v3` | 1.5 GB | ~1.5 GB | exzellent |

★ Standard beim ersten Start: wird automatisch beim App-Launch heruntergeladen (Fortschrittsanzeige im Tray/Overlay). Alle anderen Modelle werden nur auf expliziten Wunsch heruntergeladen.

Im Settings-Fenster: Download-Button mit Fortschrittsbalken für nicht installierte Modelle, Löschen-Button für installierte Modelle.

## Ressourcen-Profil (macOS M4)

- **Idle:** ~15 MB RAM, 0% CPU
- **Aufnahme aktiv:** ~20 MB RAM, <1% CPU (nur Audio-Capture)
- **Whisper-Inferenz:** ~900 MB RAM (Modell geladen), Metal GPU aktiv für ~1–3s
- **Nach Inferenz:** Modell bleibt im RAM für schnelle Folge-Aufnahmen

## Projektstruktur

```
dm-voice/
├── src-tauri/
│   ├── src/
│   │   ├── main.rs           # Tauri app entry, tray setup
│   │   ├── audio.rs          # cpal audio capture + amplitude
│   │   ├── transcriber.rs    # whisper-rs inference
│   │   ├── injector.rs       # CGEventPost / SendInput
│   │   ├── shortcut.rs       # global shortcut management
│   │   └── config.rs         # TOML config read/write
│   ├── Cargo.toml
│   └── tauri.conf.json
├── src/                      # Tauri WebView frontend
│   ├── overlay/
│   │   ├── index.html        # pill overlay
│   │   └── waveform.js       # bar animation logic
│   └── settings/
│       ├── index.html        # settings window
│       └── settings.js       # shortcut recorder, model list
└── docs/
    └── superpowers/specs/
        └── 2026-05-09-speech-to-text-design.md
```

## Abgrenzung (nicht im Scope)

- Cloud-STT-Modelle (ausschließlich lokal)
- iOS / Android
- Mehrsprachige UI (nur Deutsch/Englisch gemischt ist ok)
- Transkriptions-Verlauf / Historie
- Wörterbuch / Custom Vocabulary (kann später ergänzt werden)
