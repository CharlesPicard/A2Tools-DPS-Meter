# Your own A2 Tools site: step by step

This guide takes you from nothing to a working self-hosted A2 Tools site. When you're done, you and your friends can upload fights from the DPS meter to **your** server instead of a2tools.app.

There are two parts:

1. **The server** (`server/` folder): the website with accounts, fight logs, leaderboards, class stats and boss timers.
2. **The meter** (the desktop app): it has a new **Server** setting so it talks to your site.

The full reference for the server is [`server/README.md`](server/README.md). This file is the "do this, then this" version.

---

## Before you start: what has been tested

- ✅ Sign-in from the meter, boss timers, live updates, party presence and dev-log uploads were tested against a running server.
- ✅ Every web page was checked in a browser, on desktop and phone sizes, with made-up fights.
- ✅ The meter's 388 tests pass with the changes.
- ⚠️ **Not tested yet:** uploading a *real* fight (there were no real recordings to test with), building the Docker image (Docker Hub refused downloads at the time), and seeing the new Server box in the running desktop app.

So step 9 below (your first real upload) is the real test. If it fails, note the error message the meter shows.

---

## Step 1: Merge the changes into your fork

The work is on the branch `self-hosted-server` of `CharlesPicard/A2Tools-DPS-Meter`.

1. Open https://github.com/CharlesPicard/A2Tools-DPS-Meter/pull/new/self-hosted-server
2. Click **Create pull request**, then **Merge pull request**.

You can also skip merging and use the branch directly: in the commands below, add `-b self-hosted-server` to `git clone`.

---

## Step 2: Get a server

Any Linux machine that stays on will do:

- **A small VPS** (Hetzner, OVH, DigitalOcean, Linode…). 1 CPU and 1–2 GB of RAM is plenty for a guild. Ubuntu 24.04 is a good choice.
- **A PC or NAS at home.** This works too, but friends outside your house can only reach it if you open a port on your router. A VPS is simpler.

You also need a **domain name** (or a subdomain of one you own), for example `logs.example.org`. Without one you can't get HTTPS, and the meter will only talk to plain `http://` addresses on your own home network.

### Point the domain at the server

At your domain registrar (or Cloudflare), add a DNS record:

| Type | Name | Value |
| --- | --- | --- |
| A | `logs` | your server's IP address |

Wait a few minutes, then check it from your PC: `ping logs.example.org` should show your server's IP.

---

## Step 3: Install Docker on the server

Connect to the server (`ssh root@your-server-ip`), then:

```bash
curl -fsSL https://get.docker.com | sh
docker --version
```

---

## Step 4: Download the code

```bash
git clone https://github.com/CharlesPicard/A2Tools-DPS-Meter.git
cd A2Tools-DPS-Meter/server
```

The **whole** repository is needed, not just `server/`: the server builds in the meter's own parser so it can re-check every fight.

---

## Step 5: Configure it

Edit `docker-compose.yml`:

```bash
nano docker-compose.yml
```

Change these lines:

```yaml
      A2_PUBLIC_URL: "https://logs.example.org"   # your domain, with https://
      A2_SITE_NAME: "Charles's A2 Tools"          # any name you like
      A2_OPEN_REGISTRATION: "true"                # set to "false" later to stop new sign-ups
      A2_TRUST_PROXY: "true"                      # true, because Caddy (step 7) sits in front
```

So that only Caddy can reach the site, change the `ports` line from `"8080:8080"` to:

```yaml
      - "127.0.0.1:8080:8080"
```

Save with `Ctrl+O`, `Enter`, then quit with `Ctrl+X`.

---

## Step 6: Start the server

```bash
docker compose up -d --build
```

The first build takes a few minutes, because it compiles the server. Check it is running:

```bash
docker compose ps
curl http://127.0.0.1:8080/healthz      # should print: ok
```

If the build fails with **429 Too Many Requests**, Docker Hub is rate-limiting you. Wait a while and run the command again, or run `docker login` with a free Docker Hub account first.

To see the server's log: `docker compose logs -f` (`Ctrl+C` to stop watching).

---

## Step 7: Turn on HTTPS with Caddy

[Caddy](https://caddyserver.com) gets and renews the certificate for you automatically.

```bash
apt install -y caddy
```

Replace the contents of `/etc/caddy/Caddyfile` with your domain (`nano /etc/caddy/Caddyfile`):

```
logs.example.org {
    reverse_proxy 127.0.0.1:8080
}
```

Then:

```bash
systemctl reload caddy
```

Open the firewall for the web if you use one: `ufw allow 80,443/tcp`.

Now open **https://logs.example.org** in your browser. You should see the site.

---

## Step 8: Create your admin account

1. On the site, click **Create account**.
2. Pick a username and a password (at least 8 characters).

**The first account becomes the admin.** You'll see an **Admin** link at the top. Create yours before you tell anyone the address.

---

## Step 9: Build the meter on your Windows PC

The official meter downloads don't have the Server setting, so build the meter from your fork.

### Install the tools (once)

1. [Rust](https://rustup.rs/): run the installer and accept the defaults.
2. [Node.js](https://nodejs.org/) (the LTS version).
3. [Npcap](https://npcap.com), with **"Install Npcap in WinPcap API-compatible Mode"** checked.
4. [Git for Windows](https://git-scm.com/download/win).

### Build it

Open **PowerShell** and run:

```powershell
git clone https://github.com/CharlesPicard/A2Tools-DPS-Meter.git
cd A2Tools-DPS-Meter
npm install
npm run tauri build
```

The first build takes a while. The installer ends up in `src-tauri\target\release\bundle\msi\`. Run it to install the meter.

Linux builds work the same way; see [`docs/linux.md`](docs/linux.md).

---

## Step 10: Point the meter at your site and sign in

1. Start the meter **as Administrator**.
2. Open **Settings → A2 Tools Account**.
3. In **Server**, type `https://logs.example.org` and press **Enter**.
   - The box turns red if the address isn't accepted. It must start with `https://`; plain `http://` only works for this PC or your home network.
   - Leave it empty to go back to a2tools.app.
4. Click **Sign in**. Your browser opens your site's **Connect a meter** page with the code already filled in.
5. Sign in to the site if asked, check that the code matches the one in the meter, and click **Allow**.
6. Within a few seconds the meter shows **Signed in as …**.

Changing the Server later signs the meter out. That's deliberate: a login from one server is never sent to another.

---

## Step 11: Upload your first fight

1. Kill a boss in game.
2. Open the meter's **History**, select the fight and click the **upload** button. The share link is copied to your clipboard.
3. Open the link: you should see the fight with damage, skills, DPS chart, healing and deaths.

If you want every boss fight uploaded automatically, turn on **Upload boss fights automatically** in the same settings section.

If the upload fails, the meter shows why. Also look at `docker compose logs` on the server. The most likely problems:

| Message | What it means |
| --- | --- |
| "Not a … Evidence Slice" | The server couldn't read the recording. Tell me; this is the untested part. |
| "The recording contains no boss fight" | The fight wasn't against a boss. |
| "Your sign-in has expired" | Sign in again in Settings. |
| "Could not reach logs.example.org" | The meter can't reach the server: check the address, Caddy and the firewall. |

---

## Step 12: Invite your friends

Each friend:

1. Installs the meter **you built** (send them the `.msi` from step 9).
2. Creates an account on your site.
3. Does step 10 on their PC.

When everyone has an account, you can close sign-ups: set `A2_OPEN_REGISTRATION: "false"` in `docker-compose.yml` and run `docker compose up -d` again. As admin you can still make someone an admin, set passwords and delete accounts from the **Admin** page.

New logs are **public** by default. Each player can change that on their **Account** page (public, unlisted = link only, private = only them), and per log on the log's **About this log** tab.

---

## What's on the site

| Page | What it shows |
| --- | --- |
| **Logs** | Every public fight, filterable by boss and class; "Only mine" for your own. |
| **A log** | Damage per player, skills per player (click a player), DPS over time, healing, deaths, and a download of the recording. |
| **Leaderboards** | Top DPS per boss (by class, by period) and fastest kills. |
| **Class stats** | How each class's DPS is spread across ranked kills. |
| **Boss timers** | Field bosses per server, as meters report them. "Confirmed" means two players agree. |
| **Account** | Your name on logs, default visibility, password, and the meters signed in (you can sign one out). |
| **Admin** | Accounts, dev logs players sent, and **Re-derive logs** (use it after updating the server's parser). |

Only kills of a boss from a complete recording count for leaderboards and class stats; other logs are marked *Unranked*. Other players' names are masked (`Ts**i`); only the uploader's own name shows.

Three things still come from the official services, because they're signed with keys only the meter's developer has: the **plugin catalog**, the **supporter list** and **update checks**.

---

## Keeping it running

### Back up

Everything lives in the Docker volume `a2tools-data` (database, recordings, dev logs). To back it up:

```bash
cd ~/A2Tools-DPS-Meter/server
docker compose stop
docker run --rm -v server_a2tools-data:/data -v "$PWD":/backup debian tar czf /backup/a2tools-backup.tgz -C /data .
docker compose start
```

Copy `a2tools-backup.tgz` somewhere safe. (`docker volume ls` shows the volume's exact name if it differs.)

### Update

When the code changes (for example after pulling new meter versions from the original project):

```bash
cd ~/A2Tools-DPS-Meter
git pull
cd server
docker compose up -d --build
```

Then, on the site, go to **Admin → Re-derive logs** so old fights are recalculated with the new parser.

### Pull updates from the original meter

Your fork doesn't update itself. To bring in the original developer's new versions:

```bash
git remote add upstream https://github.com/taengu/A2Tools-DPS-Meter.git   # once
git fetch upstream
git merge upstream/main
git push
```

Rebuild the meter (step 9) and the server (Update, above) afterwards.

---

## Quick troubleshooting

| Problem | Fix |
| --- | --- |
| The site doesn't open | `docker compose ps` (is it running?), `systemctl status caddy`, and check that the DNS points at the server. |
| HTTPS certificate error | Caddy needs ports 80 and 443 open, and the DNS record pointing at the server. `journalctl -u caddy` shows why it failed. |
| The meter's browser page says "No such code" | The code expired (15 minutes). Click **Sign in** in the meter again. |
| Links from the meter point to `localhost` | `A2_PUBLIC_URL` isn't set to your domain: fix it in step 5 and run `docker compose up -d`. |
| Can't create accounts | `A2_OPEN_REGISTRATION` is `"false"`. As admin, set it back, or create the account and set its password from **Admin**. |
| Forgot the admin password | Another admin can set a new one on the **Admin** page. Make a second admin account early so this is possible. (`A2_ADMIN_USER`/`A2_ADMIN_PASSWORD` only create an admin on a site with no accounts yet.) |
