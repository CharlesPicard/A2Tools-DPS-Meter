# A2Tools DPS Meter: Self-Hosted Edition

[![License: GPL-3.0](https://img.shields.io/badge/License-GPL--3.0-blue.svg)](LICENSE)

A real-time DPS meter for AION 2, with a self-hosted web service for fight logs, leaderboards, class statistics and field boss timers.

This repository is a fork of [taengu/A2Tools-DPS-Meter](https://github.com/taengu/A2Tools-DPS-Meter). It adds a server you run yourself (`server/`) and a setting that points the meter at it. Without that setting, the meter works exactly as the original and uses [a2tools.app](https://a2tools.app).

## Contents

- [Overview](#overview)
- [Meter features](#meter-features)
- [Self-hosted server](#self-hosted-server)
- [Requirements](#requirements)
- [Installation](#installation)
- [Connecting the meter to a server](#connecting-the-meter-to-a-server)
- [Streaming to OBS on another PC](#streaming-to-obs-on-another-pc)
- [Building from source](#building-from-source)
- [Troubleshooting](#troubleshooting)
- [License and credits](#license-and-credits)

## Overview

The meter reads AION 2's network traffic on the player's PC and shows damage, skills and combat statistics in an always-on-top overlay. It does not modify the game or its memory.

Fights can be uploaded to a web service, which re-calculates every number from the fight's recorded packets with the meter's own parser. Published figures therefore reflect what the game sent, not values supplied by a client.

| Component | Location | Purpose |
| --- | --- | --- |
| Desktop meter | `src-tauri/`, `public/`, `src/` | Packet capture, live overlay, fight history, uploads |
| Self-hosted server | `server/` | Accounts, fight logs, leaderboards, class statistics, boss timers |
| Log service | `log-service/` | The original project's Cloudflare Worker for a2tools.app (not needed for self-hosting) |

## Meter features

- Real-time DPS per player, with a per-skill breakdown: critical, back attack, parry, double and perfect rates, plus optional block, perfect block, endurance, regeneration, miss and resist columns
- Damage-over-time tracking, and summon damage merged with its owner
- Target modes: Boss, Last Hit, All Targets and Train
- DPS chart and fight timeline
- Fight history, with automatic saving of boss fights
- Ping monitoring
- Stream overlay for OBS running on another PC
- Fight uploads with a shareable link, manual or automatic
- Ten languages: English, Korean, Chinese (Traditional and Simplified), Japanese, German, French, Spanish, Portuguese and Russian
- Windows and Linux ([Linux guide](docs/linux.md))

## Self-hosted server

The server provides the same services to the meter as a2tools.app, on infrastructure you control. It is a single binary with an SQLite database, and ships with a Dockerfile and a Compose file.

| Feature | Description |
| --- | --- |
| Accounts | Site accounts, and meter sign-in with the device-code flow (RFC 8628). Each signed-in meter can be revoked from the account page. |
| Fight logs | Uploads are re-derived on the server from their recording. Logs can be public, unlisted or private, and each has a shareable link. |
| Log pages | Damage and DPS per player and per skill, DPS over time, healing and deaths. The original recording can be downloaded so anyone can verify a log. |
| Leaderboards | Top DPS per boss, filterable by class and period, and fastest kills. |
| Class statistics | DPS distribution per class: median, mean, interquartile range, 95th percentile and best. |
| Field boss timers | Aggregated from players' reports, with confirmation when two reporters agree. Also served as a feed and a live WebSocket for meter plugins. |
| Administration | User management, re-derivation of all logs after a parser update, and diagnostic logs submitted from meters. |

The plugin catalog, the supporter list and update checks remain on the original project's services, because they are signed with keys held by the original developer.

Quick start:

```bash
git clone https://github.com/CharlesPicard/A2Tools-DPS-Meter.git
cd A2Tools-DPS-Meter/server
docker compose up -d --build
```

Then open `http://<server>:8080` and create the first account, which becomes the administrator.

Documentation:

- [server/README.md](server/README.md): configuration reference, HTTPS, reverse proxies, data and API
- [readme_charles.md](readme_charles.md): step-by-step deployment guide, from a new server to a working site

## Requirements

**Windows**

- Windows 10 or 11 (x86_64)
- [Npcap](https://npcap.com), installed with **WinPcap API-compatible Mode** enabled
- Administrator privileges, required for packet capture

**Linux** (playing through Proton)

- x86_64 with WebKitGTK 4.1: Ubuntu 22.04 or later, Debian 12, Fedora 39 or later, openSUSE, Arch-based distributions, Bazzite, or Steam Deck through distrobox
- Packages and setup for each distribution are in the [Linux guide](docs/linux.md)

**Server**

- Any Linux host with Docker, or a Rust toolchain to build the binary
- A domain name and HTTPS for players outside your local network

## Installation

1. Install [Npcap](https://npcap.com) with WinPcap API-compatible Mode enabled.
2. Build the installer as described in [Building from source](#building-from-source), or use one provided by your server's administrator.
3. Run the installer.
4. Start A2Tools DPS Meter as Administrator.

The official builds from the [original project's releases](https://github.com/taengu/A2Tools-DPS-Meter/releases) work with a2tools.app but do not include the server setting.

## Connecting the meter to a server

1. Open **Settings → A2 Tools Account**.
2. Enter the server's address under **Server**, for example `https://logs.example.org`, and press Enter.
3. Select **Sign in**. The browser opens the server's sign-in page with a code; approve it while signed in to the site.

Notes:

- Addresses must use `https://`. Plain `http://` is accepted only for the local computer or the local network.
- Leave the field empty to use a2tools.app.
- Changing the server signs the meter out, so a token issued by one server is never sent to another.
- The `A2TOOLS_API` environment variable sets the server as well; the setting takes precedence.

## Streaming to OBS on another PC

When OBS runs on a second PC, the meter can serve its display over the local network:

1. On the game PC, open **Settings → Streaming** and enable **Stream overlay (OBS on another PC)**. The default port is 18731.
2. When Windows Defender Firewall asks, allow the meter on **Private networks**. The meter never adds a firewall rule itself.
3. Copy the URL shown (`http://<game PC's LAN IP>:18731/overlay?key=<key>`).
4. In OBS on the streaming PC, add a **Browser Source** with that URL.

The overlay is read-only and reachable only with its key; **New key** invalidates all previous links. Optional URL parameters:

| Parameter | Effect |
| --- | --- |
| `&lang=ko` (or `en`, `de`, `ja`, …) | Language of the overlay's labels |
| `&rows=5` | Maximum number of rows shown |

**Your name on stream** sets the name shown for you; leave it empty to show your character name.

## Building from source

### Prerequisites

- [Rust](https://rustup.rs/), latest stable
- [Node.js](https://nodejs.org/) 18 or later
- [Npcap](https://npcap.com) (Windows)

### Meter

```bash
npm install
npm run tauri build
```

The Windows installer is written to `src-tauri/target/release/bundle/msi/`. For development, use `npm run tauri dev`.

### Offline build

A build without any online features (account, uploads, update checks, diagnostic log submission, Discord activity and the OBS stream overlay):

```bash
npm install
npm run build:offline
```

This omits the `online` Cargo feature, so the network client is not linked at all. Only skill and buff icons, loaded from the game's own CDN, and links you open yourself leave the computer.

### Server

```bash
cargo build --release --manifest-path server/Cargo.toml
```

### Tests

```bash
cd src-tauri && cargo test --lib     # meter
cd server && cargo test              # server
node --test tests/*.test.mjs         # frontend
```

## Troubleshooting

| Symptom | Resolution |
| --- | --- |
| The meter shows "Connecting..." | Make sure AION 2 is running and the meter has administrator privileges. VPNs and ping reducers are detected automatically through the loopback adapter. |
| Your name does not appear | Enter your character name and actor ID in Settings. The name is otherwise detected from the AION 2 window title. |
| Npcap is installed but capture fails | Reinstall Npcap with WinPcap API-compatible Mode enabled. |
| The Server field turns red | The address must start with `https://`, or `http://` for a local address. |
| Sign-in page reports an unknown code | The code expired after 15 minutes. Select **Sign in** again in the meter. |

Server issues are covered in [server/README.md](server/README.md) and [readme_charles.md](readme_charles.md).

## License and credits

Licensed under the [GNU General Public License v3.0](LICENSE). Modified versions must remain under the same license.

- The meter is the work of [taengu](https://github.com/taengu) and the contributors to [A2Tools-DPS-Meter](https://github.com/taengu/A2Tools-DPS-Meter).
- Some parser and storage fixes, the buff and debuff parser (`src-tauri/src/capture/abnormal.rs`) and its data tables (`src/data/abnormals.json`, `src/data/i18n/abnormals/`) come from [Daevalog](https://github.com/Seralth/Daevalog), Seralth's GPL-3.0 fork. Each ported change is committed under Seralth's name with a `Ported-from:` line.
- Bundled fonts are licensed under the SIL Open Font License 1.1: [Noto Sans SC](public/vendor/fonts/noto-sans-sc/LICENSE), [Noto Sans TC](public/vendor/fonts/noto-sans-tc/LICENSE) and [Pretendard](public/vendor/fonts/pretendard/LICENSE).

AION 2 is a trademark of NCSOFT. This project is not affiliated with or endorsed by NCSOFT.
