# Self-hosted A2 Tools server

Your own copy of [a2tools.app](https://a2tools.app) for the A2Tools DPS Meter: one program and one data folder, on your own server.

It does what a2tools.app does for the meter:

| Feature | What it does |
| --- | --- |
| **Accounts and meter sign-in** | Players make an account on the site and sign the meter in with a code (the same device-code flow the meter already uses with a2tools.app). Each signed-in meter is listed on the account page and can be signed out from there. |
| **Fight logs** | The meter uploads a fight's recording (its Evidence Slice). The server runs **the meter's own parser** on it, so a log shows what the game's packets say, not numbers a client typed in. You get a link to share. Logs can be public, unlisted or private. |
| **Log pages** | Per player: damage, DPS, share, crit, back attack and perfect rates. Per skill for each player. DPS over time, healing and deaths. The recording can be downloaded so anyone can check a log. |
| **Leaderboards** | Top DPS per boss (filter by class and period) and fastest kills, from ranked public logs. |
| **Class stats** | DPS spread per class (median, mean, middle half, top 5%, best), for all bosses or one. |
| **Field boss timers** | Meters report what their map shows, kills and boss HP. The server combines the reports per server and marks an entry confirmed when two players agree. They are published on the site, as the feed plugins read (`/feeds/boss-timers/<server>.json`) and as live updates over a WebSocket (`/api/boss-timers/live`). |
| **Party presence** | The marks the meter shows next to party members who use the meter. Only hashes of a name and server are sent. |
| **Send logs to dev** | Packet logs players send from the meter for debugging. Only admins can see them; they are deleted after 30 days. |
| **Admin** | Accounts (admin, supporter, password reset, delete), dev logs, and re-deriving every log after the parser changes. |

The site is in English, built into the binary, and works on phones.

Some things stay with the official services, because they are signed with keys only the meter's developer holds: the plugin catalog, the supporter roster and the meter's update check. A meter pointed at your server keeps using those as before.

## Run it

### Docker

From the repository root (the server compiles in the meter's parser from `src-tauri/`):

```bash
cd server
docker compose up -d
```

Open `http://<your server>:8080` and create an account. **The first account becomes the admin.** Set the environment in `docker-compose.yml` first (below), at least `A2_PUBLIC_URL`.

### Without Docker

You need [Rust](https://rustup.rs/) (stable):

```bash
cargo build --release --manifest-path server/Cargo.toml
A2_PUBLIC_URL=https://logs.example.org A2_DATA_DIR=/var/lib/a2tools ./server/target/release/a2tools-server
```

A systemd unit, for example `/etc/systemd/system/a2tools.service`:

```ini
[Unit]
Description=A2 Tools server
After=network.target

[Service]
User=a2tools
Environment=A2_LISTEN=127.0.0.1:8080
Environment=A2_PUBLIC_URL=https://logs.example.org
Environment=A2_DATA_DIR=/var/lib/a2tools
Environment=A2_TRUST_PROXY=true
ExecStart=/usr/local/bin/a2tools-server
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

### HTTPS

The meter sends its sign-in token with every upload, so for players outside your own network the server must be reached over **https**. The meter refuses plain `http://` except for this computer or the local network (`localhost`, `192.168.x.x`, `10.x.x.x`, `*.local`, …).

Put a reverse proxy in front and set `A2_TRUST_PROXY=true`. With [Caddy](https://caddyserver.com), which gets the certificate itself:

```
logs.example.org {
    reverse_proxy 127.0.0.1:8080
}
```

With nginx, pass WebSocket upgrades for the live boss timers, and allow uploads up to 24 MB:

```nginx
location / {
    proxy_pass http://127.0.0.1:8080;
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    client_max_body_size 41m;
}
```

## Settings

All settings are environment variables:

| Variable | Default | |
| --- | --- | --- |
| `A2_PUBLIC_URL` | `http://localhost:8080` | The address players reach the site at. The sign-in page and log links are built from it. |
| `A2_LISTEN` | `0.0.0.0:8080` | Address and port to listen on. |
| `A2_DATA_DIR` | `./data` | Database, recordings and dev logs. **Back this up.** |
| `A2_SITE_NAME` | `A2 Tools` | The name shown in the site's pages. |
| `A2_OPEN_REGISTRATION` | `true` | Whether anyone can make an account. The first account can always be made, and becomes the admin. |
| `A2_ADMIN_USER`, `A2_ADMIN_PASSWORD` | | Create the admin account at first start, instead of registering it on the site. |
| `A2_TRUST_PROXY` | `false` | Take the client address from `X-Forwarded-For`. Turn it on only behind a reverse proxy. |
| `A2_UPLOADS_PER_HOUR` | `120` | Uploads one account may make per hour. |
| `A2_DEV_LOG_DAYS` | `30` | Days dev logs are kept. |
| `RUST_LOG` | `info` | Log level. |

## Point the meter at it

In the meter, open **Settings → A2 Tools Account** and enter your server's address under **Server**, for example `https://logs.example.org`. Then press **Sign in**: the browser opens your site's `/link` page with the code filled in, and you approve it while signed in there.

Leave **Server** empty to go back to a2tools.app. Changing it signs the meter out, so a token from one server is never sent to another.

You can also set the `A2TOOLS_API` environment variable before starting the meter; the setting takes precedence.

With the server set, the meter sends everything that used to go to a2tools.app to your server: sign-in, fight uploads (manual and automatic), boss timer reports, party presence, Send logs to dev, and the boss timer feed and live updates for plugins.

## What is stored

- `a2tools.db`: an SQLite database with accounts (passwords hashed with Argon2), sign-ins (stored only as hashes), logs and their players, presence hashes and dev-log reports.
- `slices/`: each log's recording, kept so it can be re-derived when the parser changes (Admin → Re-derive logs).
- `records/`: each log's derived fight.
- `devlogs/`: packet logs sent with Send logs to dev.
- `boss-timers.json`: the latest field boss timers per server.

Player names in a recording are blinded by the meter before upload. A log shows the uploader's own name and masks everyone else's (`Ts**i`), as on a2tools.app.

## Ranked logs

A log counts toward the leaderboards and class stats when it is public and the server finds it to be:

- a kill of the boss, and not a training dummy;
- a complete recording: the damage found reaches at least 95% of the boss's HP, and the packets were read in their proper framing;
- free of player names in plain text.

Other logs are still kept and shown; they are marked *Unranked*.

## API

The routes the meter uses are the same as a2tools.app's: `POST /api/device/code`, `POST /api/device/token`, `GET /api/me`, `POST /api/logs`, `POST /api/presence`, `POST /api/boss-timers/{report,kill,hp,cores,artifacts}`, `GET /api/boss-timers/live` (WebSocket), `GET /feeds/boss-timers/<server>.json`, `POST /api/dev-logs` and `PUT /api/dev-logs/<code>/<file>`. The site's pages read `GET /api/logs`, `/api/logs/<id>`, `/api/leaderboards`, `/api/stats/classes`, `/api/bosses`, `/api/boss-timers` and `/api/overview`.
