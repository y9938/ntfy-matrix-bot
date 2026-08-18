# ntfy-matrix-bot

E2EE Matrix bot for provisioning users and ACLs on self-hosted [ntfy](https://ntfy.sh).

## Prerequisites

- Docker and Docker Compose
- Self-hosted ntfy server (for Admin API access)
- Matrix account for the bot

## Deployment

### 1. Configure Environment

```bash
cp .env.example .env
mkdir -p secrets
```

Edit `.env` to set credentials and required ACLs (`MATRIX_ALLOWED_USERS` / `MATRIX_ALLOWED_HOMESERVERS`).

### 2. Matrix Authentication (Choose ONE)

The bot requires exactly one authentication method. Create the corresponding file in the `secrets/` directory:

**Option A: Password (Default)**
Create `./secrets/matrix_password.txt` and paste your bot's password inside.
*Note: The bot uses the password only on the first run, securely caching the session.*

**Option B: Access Token**
1. Create a bot user (e.g. `!admin users create-user ntfy-bot <pass>` in Tuwunel `#admins` room).
2. Generate an access token via Admin API:
   ```bash
    HOMESERVER="https://matrix.example.com" BOT_USER_ID="@ntfy-bot:example.com" ADMIN_TOKEN="..." \
   bash -c 'curl -s -X POST "$HOMESERVER/_synapse/admin/v1/users/$BOT_USER_ID/login" -H "Authorization: Bearer $ADMIN_TOKEN" -H "Content-Type: application/json" -d "{}" | jq -r .access_token'
   ```
3. Create `./secrets/matrix_access_token.txt` and paste the token inside.
4. Edit `compose.yaml`: Switch from Password to Access Token secret.

### 3. ntfy Admin Token

Generate an admin token on your ntfy server:
```bash
docker exec ntfy ntfy token add --label "ntfy-matrix-bot" <admin-username>
```
Create `./secrets/ntfy_token.txt` and paste the token inside.

### 4. Run

Secure the credentials and start the container:
```bash
sudo chown -R 65532:65532 secrets
chmod 600 secrets/*
docker compose up -d
```

> [!WARNING]
> The database volume (`data` -> `/data`) stores SQLite state and E2EE Megolm keys. Do not delete it, or the bot will lose access to encrypted rooms.
