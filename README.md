# A2Tools DPS Meter: Self-Hosted Edition for CachyOS

[![License: GPL-3.0](https://img.shields.io/badge/License-GPL--3.0-blue.svg)](LICENSE)

A real-time DPS meter for AION 2 on CachyOS, with a self-hosted web service for fight logs, leaderboards, class statistics and field boss timers.

This repository is a fork of [taengu/A2Tools-DPS-Meter](https://github.com/taengu/A2Tools-DPS-Meter). It adds a server you run yourself (`server/`) and a setting that points the meter at it. Without that setting, the meter behaves like the original and uses [a2tools.app](https://a2tools.app).

These instructions are written for **CachyOS**. They also apply to Arch Linux and other Arch-based distributions.

## Contents

- [Overview](#overview)
- [Requirements](#requirements)
- [Install the meter](#install-the-meter)
- [Run the meter](#run-the-meter)
- [Run the server](#run-the-server)
- [Connect the meter to the server](#connect-the-meter-to-the-server)
- [Update](#update)
- [Uninstall](#uninstall)
- [Streaming to OBS on another PC](#streaming-to-obs-on-another-pc)
- [Development](#development)
- [Troubleshooting](#troubleshooting)
- [License and credits](#license-and-credits)

## Overview

The meter reads AION 2's network traffic, with the game running through Proton, and shows damage, skills and combat statistics in an always-on-top overlay. It does not modify the game or its memory.

Fights can be uploaded to the server, which re-calculates every number from the fight's recorded packets with the meter's own parser. Published figures therefore reflect what the game sent, not values supplied by a client.

**Meter**

- Real-time DPS per player, with a per-skill breakdown: critical, back attack, parry, double and perfect rates, plus optional block, perfect block, endurance, regeneration, miss and resist columns
- Damage-over-time tracking, and summon damage merged with its owner
- Target modes: Boss, Last Hit, All Targets and Train
- DPS chart, fight timeline and fight history, with automatic saving of boss fights
- Ping monitoring and a stream overlay for OBS
- Fight uploads with a shareable link, manual or automatic
- Ten languages

**Server**

| Feature | Description |
| --- | --- |
| Accounts | Site accounts, and meter sign-in with a device code. Each signed-in meter can be revoked from the account page. |
| Fight logs | Uploads are re-derived on the server from their recording. Logs can be public, unlisted or private, and each has a shareable link. |
| Log pages | Damage and DPS per player and per skill, DPS over time, healing and deaths, and a download of the original recording. |
| Leaderboards | Top DPS per boss, filterable by class and period, and fastest kills. |
| Class statistics | DPS distribution per class: median, mean, interquartile range, 95th percentile and best. |
| Field boss timers | Aggregated from players' reports, confirmed when two reporters agree, and served live to meter plugins. |
| Administration | User management, re-derivation of all logs after a parser update, and diagnostic logs submitted from meters. |

The plugin catalog, the supporter list and update checks remain on the original project's services, because they are signed with keys held by the original developer.

## Requirements

- CachyOS (x86_64) with a desktop session (KDE Plasma, GNOME or another)
- AION 2 running through Steam with Proton
- About 5 GB of free disk space for the first build
- For the server: any CachyOS or Linux machine that stays on. Players outside your local network also need a domain name and HTTPS.

## Install the meter

The meter is built from this repository into a regular pacman package. The package installs the meter to `/usr/bin`, adds it to the application menu and grants it the packet-capture permission, so it never needs to run as root.

### 1. Install the build tools

```bash
sudo pacman -S --needed base-devel git rust nodejs npm pkgconf webkit2gtk-4.1 gtk3 libpcap libcap gst-plugins-base gst-plugins-good
```

### 2. Download the source

```bash
git clone https://github.com/CharlesPicard/A2Tools-DPS-Meter.git
cd A2Tools-DPS-Meter
```

Use a full clone: the package version is computed from the Git history.

### 3. Build and install the package

```bash
cd packaging/arch
makepkg -si
```

The first build takes 10 to 15 minutes. `makepkg` asks for your password to install the package. The output should end with:

```
==> A2Tools DPS Meter may now capture packets.
```

## Run the meter

1. Start AION 2 from Steam.
2. Start **A2Tools DPS Meter** from the application menu, or run `a2tools-dps-meter` in a terminal.

Start it as your normal user. Do not run it with `sudo`: it would run as root, keep its settings in root's home folder, and often fail to open its window.

The meter detects the game's connection by itself. Its window stays above the game; use the lock button to let clicks pass through it.

## Run the server

The server is a single program with a data folder. Docker is the simplest way to run it.

### With Docker

```bash
sudo pacman -S --needed docker docker-compose
sudo systemctl enable --now docker
cd A2Tools-DPS-Meter/server
sudo docker compose up -d --build
```

### Without Docker

```bash
cd A2Tools-DPS-Meter
cargo build --release --manifest-path server/Cargo.toml
A2_PUBLIC_URL=http://<this machine's IP>:8080 ./server/target/release/a2tools-server
```

### First start

1. Open `http://<server IP>:8080` in a browser.
2. Select **Create account**. The first account becomes the administrator.

Set `A2_PUBLIC_URL` (in `server/docker-compose.yml` when using Docker) to the address players use to reach the site; the links the meter opens and shares are built from it.

For players outside your local network, put the server behind HTTPS. With [Caddy](https://caddyserver.com):

```bash
sudo pacman -S caddy
```

Write `/etc/caddy/Caddyfile`:

```
logs.example.org {
    reverse_proxy 127.0.0.1:8080
}
```

```bash
sudo systemctl enable --now caddy
```

Then set `A2_PUBLIC_URL=https://logs.example.org` and `A2_TRUST_PROXY=true`.

Full reference: [server/README.md](server/README.md). Step-by-step guide: [readme_charles.md](readme_charles.md).

## Connect the meter to the server

1. In the meter, open **Settings → A2 Tools Account**.
2. Under **Server**, enter the server's address, for example `https://logs.example.org` or `http://192.168.1.20:8080`, and press Enter.
3. Select **Sign in**. The browser opens the server's sign-in page with the code filled in; select **Allow** while signed in to the site.
4. Kill a boss, open **History**, select the fight and upload it. The link to the log is copied to the clipboard. To upload every boss fight, enable **Upload boss fights automatically**.

Notes:

- Addresses must use `https://`. Plain `http://` is accepted only for the local computer or the local network (`localhost`, `192.168.x.x`, `10.x.x.x`, `*.local`).
- Leave **Server** empty to use a2tools.app.
- Changing the server signs the meter out, so a token issued by one server is never sent to another.

## Update

### The meter

The meter's update prompt installs the **original** project's package, which does not include the server setting. Decline it, and update from this repository instead:

```bash
cd A2Tools-DPS-Meter
git pull
cd packaging/arch
makepkg -sif
```

Settings and fight history are kept. Check the installed version with `pacman -Q a2tools-dps-meter`.

To bring in new versions from the original project first:

```bash
git remote add upstream https://github.com/taengu/A2Tools-DPS-Meter.git   # once
git fetch upstream
git merge upstream/main
git push
```

### The server

```bash
cd A2Tools-DPS-Meter
git pull
cd server
sudo docker compose up -d --build
```

Then open **Admin → Re-derive logs** on the site so existing logs are recalculated with the new parser.

## Uninstall

```bash
sudo pacman -R a2tools-dps-meter
```

Settings and fight history remain in `~/.local/share/com.a2tools.dps-meter/` and can be deleted by hand.

## Streaming to OBS on another PC

When OBS runs on a second PC, the meter can serve its display over the local network:

1. Open **Settings → Streaming** and enable **Stream overlay (OBS on another PC)**. The default port is 18731.
2. If a firewall is active, allow that port from the local network, for example `sudo ufw allow from 192.168.0.0/16 to any port 18731 proto tcp`.
3. Copy the URL shown (`http://<this PC's LAN IP>:18731/overlay?key=<key>`).
4. In OBS on the streaming PC, add a **Browser Source** with that URL.

The overlay is read-only and reachable only with its key; **New key** invalidates all previous links. Add `&lang=ko` (or `en`, `de`, `ja`, …) to change the language of its labels, and `&rows=5` to limit the number of rows.

## Development

Run the meter from the source tree without installing it:

```bash
mkdir -p public/i18n public/data public/src/data
cp -r src/data/i18n/* public/i18n/
cp src/data/skill_icons.json src/data/dot_skill_ids.json public/data/
cp src/data/skill_icons.json src/data/dot_skill_ids.json public/src/data/
npm install
npx tauri build --no-bundle
sudo setcap cap_net_raw=ep src-tauri/target/release/a2tools-dps-meter
./src-tauri/target/release/a2tools-dps-meter
```

`setcap` must be run again after every build, and does not work on NTFS or exFAT drives.

Tests:

```bash
cd src-tauri && cargo test --lib     # meter
cd server && cargo test              # server
node --test tests/*.test.mjs         # frontend
```

For an offline build, without the account, uploads, update checks, diagnostic log submission, Discord activity and the stream overlay, use `npm run build:offline`.

## Troubleshooting

| Symptom | Resolution |
| --- | --- |
| The meter shows "Connecting..." | Make sure AION 2 is running. `getcap /usr/bin/a2tools-dps-meter` should print `cap_net_raw=ep`; if it does not, run `sudo setcap cap_net_raw=ep /usr/bin/a2tools-dps-meter`. |
| The meter goes behind the game | On KDE Plasma, the meter adds a KWin window rule by itself; restart the meter once. On GNOME, see the [Linux guide](docs/linux.md#gnome-keep-the-meter-above-other-windows). |
| Your name does not appear | Enter your character name and actor ID in Settings. |
| `makepkg` fails to compile | Update the system with `sudo pacman -Syu`, then run `makepkg -sif` again. |
| The Server field turns red | The address must start with `https://`, or `http://` for a local address. |
| The sign-in page reports an unknown code | The code expired after 15 minutes. Select **Sign in** again in the meter. |
| An upload fails | The meter shows the reason. On the server, `sudo docker compose logs` shows what it received. |

Hotkeys, window behaviour and sending diagnostic logs are covered in the [Linux guide](docs/linux.md).

## License and credits

Licensed under the [GNU General Public License v3.0](LICENSE). Modified versions must remain under the same license.

- The meter is the work of [taengu](https://github.com/taengu) and the contributors to [A2Tools-DPS-Meter](https://github.com/taengu/A2Tools-DPS-Meter).
- Some parser and storage fixes, the buff and debuff parser (`src-tauri/src/capture/abnormal.rs`) and its data tables (`src/data/abnormals.json`, `src/data/i18n/abnormals/`) come from [Daevalog](https://github.com/Seralth/Daevalog), Seralth's GPL-3.0 fork. Each ported change is committed under Seralth's name with a `Ported-from:` line.
- Bundled fonts are licensed under the SIL Open Font License 1.1: [Noto Sans SC](public/vendor/fonts/noto-sans-sc/LICENSE), [Noto Sans TC](public/vendor/fonts/noto-sans-tc/LICENSE) and [Pretendard](public/vendor/fonts/pretendard/LICENSE).

AION 2 is a trademark of NCSOFT. This project is not affiliated with or endorsed by NCSOFT.
