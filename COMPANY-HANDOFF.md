# TimeTracker — Company Handoff & Setup Requirements

This document lists everything needed from the company / account owner to run
TimeTracker in production and to publish **signed, notarized desktop releases**.

> ⚠️ **Never commit real secret values.** This file lists *what* is needed and
> *where* it goes — not the values. Secrets live in GitHub Actions secrets (for
> releases) and in the server's `.env` on the host (for runtime).

---

## 1. GitHub Actions secrets — required for macOS releases

Set these under **Repo → Settings → Secrets and variables → Actions → New
repository secret**. Without them the macOS build cannot be signed/notarized and
users get Gatekeeper warnings (or the build fails).

| Secret | What it is | Provided by |
|---|---|---|
| `APPLE_CERTIFICATE` | Base64 of the **Developer ID Application** `.p12` | Apple Developer account owner |
| `APPLE_CERTIFICATE_PASSWORD` | Password set when exporting that `.p12` | Same |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: <Company> (<TEAMID>)` | Same |
| `APPLE_ID` | Apple ID email of the developer account | Same |
| `APPLE_PASSWORD` | App-specific password for that Apple ID (for notarization) | Same |
| `APPLE_TEAM_ID` | 10-character Apple Developer Team ID | Same |

Windows/Git-Bash: base64 the certificate with `base64 -w0 timetracker.p12`.

## 2. How to produce the Apple materials (account owner, one-time)

1. **Team ID** — developer.apple.com → **Membership details** → copy the 10-char Team ID.
2. **App-specific password** — appleid.apple.com → **Sign-In and Security → App-Specific
   Passwords → Generate** (requires 2FA on the Apple ID). Label it "TimeTracker Notarization".
3. **Developer ID Application certificate** (only the account holder can create it):
   - Generate a Certificate Signing Request (`.certSigningRequest`) — on a Mac via
     **Keychain Access → Certificate Assistant → Request a Certificate…**, or with OpenSSL
     (`openssl genrsa -out key.pem 2048 && openssl req -new -key key.pem -out req.certSigningRequest -subj "/CN=Developer ID Application/emailAddress=<APPLE_ID>/C=US"`).
   - developer.apple.com → **Certificates → ＋ → Developer ID Application** → upload the CSR → download the `.cer`.
   - Bundle cert + private key into a password-protected `.p12`. With OpenSSL use the
     **`-legacy`** flag so the CI runner can import it:
     `openssl x509 -inform DER -in developerID_application.cer -out cert.pem`
     `openssl pkcs12 -legacy -export -out timetracker.p12 -inkey key.pem -in cert.pem`
   - The `.p12` contains a private key — transfer it securely and keep the password separate.

## 3. Production runtime secrets — host `.env` (not committed)

These already exist on the current server; the company should own/rotate them.
Set in the server host's `.env` (read by `docker compose`):

| Variable | Purpose |
|---|---|
| `DATABASE_URL` | Managed Postgres (Supabase **session** pooler, port 5432, `?sslmode=require`) |
| `JWT_ACCESS_SECRET` | ≥32 chars (`openssl rand -base64 48`) |
| `S3_ENDPOINT` / `S3_REGION` / `S3_BUCKET` / `S3_ACCESS_KEY_ID` / `S3_SECRET_ACCESS_KEY` | Object storage (Google Cloud Storage, S3-compatible) |
| `S3_ALLOW_INSECURE_DEFAULTS=false` | Enforce real storage keys in production |
| `PUBLIC_BASE_URL` | Public API base (e.g. `https://time-tracker.rapidinnovation.dev`) |
| `SMTP_HOST` / `SMTP_PORT` / `SMTP_USER` / `SMTP_PASS` / `SMTP_FROM` | Email (invites, notifications) |
| `XAI_API_KEY` (+ optional `XAI_MODEL`) **or** `ANTHROPIC_API_KEY` | Screenshot AI analysis; xAI takes precedence if set |

## 4. Accounts, ownership & funding needed

- [ ] **Apple Developer Program** membership (have) — enables signing + notarization.
- [ ] **AI provider credits** — screenshot analysis is currently blocked on funding.
      Fund the xAI team (console.x.ai) or set an Anthropic key with credit.
- [ ] **Hosting** — GCP VM (Docker + nginx) ownership/access; currently
      `time-tracker.rapidinnovation.dev`.
- [ ] **Database** — Supabase project ownership.
- [ ] **Object storage** — Google Cloud Storage bucket + service-account key.
- [ ] **DNS / CDN** — Cloudflare zone for the domain.

## 5. Deploying (server + web)

The server auto-applies DB migrations on startup. From the host repo checkout:

```bash
git fetch origin && git reset --hard origin/main
docker compose up -d --build server admin-web employee-web
docker compose restart nginx   # so nginx re-resolves the recreated containers
docker compose logs -f server  # confirm migrations applied + "listening on 0.0.0.0:9000"
```

Desktop app releases are cut by tagging (GitHub Actions builds Windows/macOS/Linux
installers and publishes them to GitHub Releases).
