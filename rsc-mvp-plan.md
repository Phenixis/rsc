# rsc — Plan MVP d'un client SoundCloud en terminal (Rust, Linux)

Client TUI minimal : authentification OAuth au compte SoundCloud, choix d'une playlist dont on est propriétaire, lecture dans l'ordre.

---

## 1. Périmètre

**Inclus**

- Login OAuth 2.1 (Authorization Code + PKCE), persistance et refresh des tokens
- Liste des playlists de l'utilisateur
- Sélection d'une playlist, lecture des pistes dans l'ordre
- Contrôles : pause, piste suivante/précédente, quitter ; ligne « now playing »

**Exclus (post-MVP)** : recherche, likes, feed, shuffle, barre de seek, MPRIS, cache, gapless, keyring.

**Estimation** : 15-25 h pour un dev à l'aise en Rust, dont l'auth est le plus gros morceau.

---

## 2. Gate 0 — À vérifier avant d'écrire du code (1-2 h)

1. **Enregistrement d'une app.** La doc développeur indique qu'un compte SoundCloud **Artist Pro** est requis pour obtenir un `client_id` / `client_secret` (via le navigateur ou la CLI d'identifiants API). Vérifier sur `soundcloud.com/you/apps` que l'enregistrement est possible pour vous.
2. **Pas de secret embarqué.** Tous les clients sont traités comme *confidentiels* : le `client_secret` est requis même pour l'échange Authorization Code + PKCE. Pour un projet open source, chaque utilisateur enregistre sa propre app et fournit ses identifiants par config. À concevoir dès le départ.
3. **Pistes lisibles.** Seules les pistes `access: playable` sont streamables hors plateforme. `preview` = extrait seulement, `blocked` = pas de `stream_url`. Une playlist de titres de majors peut être en grande partie injouable.

Si le point 1 échoue : repli sur `yt-dlp` + `mpv` (voir §12, option C). Le reste du plan (player, queue, TUI) reste valable.

---

## 3. Dépendances

```toml
[dependencies]
tokio = { version = "1", features = ["rt-multi-thread", "macros", "net", "process", "time", "sync", "io-util", "fs", "signal"] }
tokio-stream = "0.1"
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
clap = { version = "4", features = ["derive"] }
ratatui = "0.29"
crossterm = { version = "0.28", features = ["event-stream"] }
anyhow = "1"
thiserror = "2"
async-trait = "0.1"
directories = "5"
sha2 = "0.10"
base64 = "0.22"
rand = "0.8"
url = "2"
tracing = "0.1"
tracing-subscriber = "0.3"

[dev-dependencies]
wiremock = "0.6"
tempfile = "3"
```

- Pas besoin de `tiny_http` : le listener loopback se fait à la main sur `tokio::net::TcpListener` (une seule ligne de requête à parser).
- Dépendance runtime : **`mpv`** (vérifier avec `which mpv` au démarrage, message d'erreur clair sinon). `xdg-open` pour ouvrir le navigateur.

---

## 4. Architecture

Workspace Cargo à deux binaires : `rsc` (l'app) et `rsc-mock` (serveur de dev, voir §12).

```
rsc/
  src/
    main.rs        # clap, runtime tokio, câblage
    config.rs      # client_id/secret, base URLs, chemins, options mpv
    backend.rs     # trait Backend + impls (official, local)
    auth/
      mod.rs       # AuthManager: get_valid_token(), login()
      pkce.rs      # verifier / challenge / state
      callback.rs  # listener loopback
      store.rs     # lecture/écriture atomique du fichier de tokens
    api/
      mod.rs       # Client: requête authentifiée + retry sur 401
      models.rs    # Playlist, Track, Page<T>
      playlists.rs # list_mine(), get_with_tracks()
      stream.rs    # resolve_stream_url()
    player/
      mod.rs       # trait Player
      mpv.rs       # spawn + IPC JSON
    queue.rs       # file ordonnée, curseur, advance()
    app.rs         # état App + reducer d'Actions
    ui.rs          # fonctions de rendu ratatui
  rsc-mock/     # serveur de dev (axum)
```

Deux traits à isoler tôt :

```rust
#[async_trait]
pub trait Backend: Send + Sync {
    async fn login(&self) -> anyhow::Result<()>;
    async fn my_playlists(&self) -> anyhow::Result<Vec<Playlist>>;
    async fn playlist_tracks(&self, id: &PlaylistId) -> anyhow::Result<Vec<Track>>;
    async fn stream_target(&self, track: &Track) -> anyhow::Result<StreamTarget>;
}

#[async_trait]
pub trait Player: Send + Sync {
    async fn load(&self, target: &StreamTarget) -> anyhow::Result<()>;
    async fn set_pause(&self, paused: bool) -> anyhow::Result<()>;
    async fn stop(&self) -> anyhow::Result<()>;
}
```

`Backend` permet de brancher l'API officielle, un backend local ou un mock ; `Player` permet un `FakePlayer` pour les tests.

---

## 5. Authentification

### Redirect URI

Loopback fixe : `http://127.0.0.1:8888/callback`, enregistré tel quel dans les réglages de l'app. Le `redirect_uri` doit correspondre **exactement** à celui de la requête d'autorisation : pas de `localhost` vs `127.0.0.1`, pas de slash final différent, port fixe.

Un schéma personnalisé (`my-app://…`) est possible mais exige un fichier `.desktop` et un enregistrement `xdg-mime` : surdimensionné pour un MVP. Prévoir un flag `--manual` : affiche l'URL, lit le `code` collé sur stdin (utile en headless / SSH).

### Flux

1. Générer `code_verifier` (32 octets aléatoires, base64url sans padding → 43 caractères), `code_challenge = base64url_nopad(sha256(verifier))`, méthode `S256`, plus un `state` aléatoire.
2. **Binder le `TcpListener` avant d'ouvrir le navigateur** (pas de race).
3. Ouvrir via `xdg-open` (et afficher l'URL aussi) :
   `https://secure.soundcloud.com/authorize?client_id=…&redirect_uri=…&response_type=code&code_challenge=…&code_challenge_method=S256&state=…`
4. Accepter **une** connexion, parser la ligne de requête, extraire `code` et `state`, vérifier `state`, répondre une petite page HTML, fermer le listener. Timeout global de 5 min (`tokio::time::timeout`).
5. `POST https://secure.soundcloud.com/oauth/token` en `application/x-www-form-urlencoded` :
   `grant_type=authorization_code`, `client_id`, `client_secret`, `redirect_uri`, `code_verifier`, `code`.
   Réponse : `access_token`, `refresh_token`, `expires_in`, `scope`.
6. Persister avec `expires_at = now + expires_in - 60 s` (marge).

Toutes les requêtes API portent `Authorization: OAuth <access_token>` (**`OAuth`, pas `Bearer`**).

### Refresh

Les access tokens expirent au bout d'environ 1 h et **chaque refresh token est à usage unique**. C'est la source classique de déconnexions inexpliquées :

- `POST /oauth/token` avec `grant_type=refresh_token`, `client_id`, `client_secret`, `refresh_token`.
- Écrire **immédiatement** le nouveau refresh token sur disque, de façon atomique (`tokens.json.tmp` → `fsync` → `rename`). Un crash entre l'usage et la persistance déconnecte l'utilisateur définitivement.
- Sérialiser les refresh derrière un `tokio::sync::Mutex` dans `AuthManager` ; à l'intérieur du lock, **revérifier l'expiration** avant de rafraîchir (deux 401 concurrents ne doivent pas brûler deux fois le même token).
- Sur `401` : un refresh, un retry. Un second `401` → redemander un login.

### Stockage

- Tokens : `~/.local/share/rsc/tokens.json`, mode `0600` (`OpenOptions::mode(0o600)`), via `directories`.
- Config : `~/.config/rsc/config.toml` (`client_id`, `client_secret`, `api_base`, `auth_base`), mode `0600`. Surcharges par variables d'environnement : `RSC_CLIENT_ID`, `RSC_CLIENT_SECRET`, `RSC_API_BASE`, `RSC_AUTH_BASE`.
- `keyring` (libsecret) : plus tard, il tire une dépendance D-Bus.

---

## 6. Couche API

Base : `https://api.soundcloud.com` (auth : `https://secure.soundcloud.com`).

### Playlists de l'utilisateur

```
GET /me/playlists?show_tracks=false&linked_partitioning=true&limit=50
```

Défaut 50 éléments, maximum 200. Avec `linked_partitioning=true`, la réponse est `{ "collection": [...], "next_href": "..." }` ; suivre `next_href` jusqu'à son absence. Un seul helper générique :

```rust
async fn collect_all<T: DeserializeOwned>(&self, first: Url) -> Result<Vec<T>>
```

avec un plafond de pages (20) contre les boucles infinies. Modéliser `Page<T>` (et non un tableau nu).

### Pistes d'une playlist

`GET /playlists/{id}`. Vérifier sur votre plus grosse playlist si la liste `tracks` est complète ou tronquée (IDs seuls) ; en cas de troncature, hydrater par lots avec `GET /tracks?ids=1,2,3`. Prévoir ~1 h : c'est la surprise la plus probable de cette couche.

### Résolution du stream

La doc mentionne à la fois la propriété `stream_url`, un exemple `GET /tracks/{id}/stream` et du texte sur `/tracks/:id/streams`. Trancher **empiriquement avec curl** et avec l'OpenAPI (`github.com/soundcloud/api`, `openapi/api.yaml`) avant de coder, et isoler dans une seule fonction :

```rust
async fn resolve_stream_url(&self, track: &Track) -> Result<StreamTarget>
```

- Résoudre **paresseusement, juste avant la lecture** de chaque piste, jamais au chargement de la playlist (URLs à durée de vie courte).
- Vérifier si l'URL de stream exige l'en-tête `Authorization: OAuth …` ; si oui, le passer à mpv (`--http-header-fields`).
- Le `StreamTarget` peut être progressif ou HLS ; mpv gère les deux.

### Filtrage et conformité

- Ignorer les pistes `blocked` à la construction de la queue et les marquer dans l'UI (une playlist de 30 pistes peut n'en jouer que 22). Ne jamais prolonger une piste `preview`.
- **Attribution obligatoire** dans le now-playing : titre, uploader (créateur), `permalink_url` (lien vers l'œuvre), mention « via SoundCloud ».
- Pas de téléchargement ni de cache audio.
- Limite : 15 000 requêtes de stream par 24 h et par `client_id` (sans objet en usage perso ; logguer un warning sur `429`).

---

## 7. Lecture (mpv via IPC JSON)

Lancer une fois au démarrage :

```
mpv --idle=yes --no-video --no-terminal --really-quiet \
    --input-ipc-server=$XDG_RUNTIME_DIR/rsc-$PID.sock
```

Se connecter avec `tokio::net::UnixStream` (retry ~2 s : la socket apparaît de façon asynchrone). Protocole JSON délimité par des sauts de ligne :

- Charger : `{"command":["loadfile", "<url>", "replace"], "request_id": 1}`
- Pause : `{"command":["set_property","pause",true]}`
- Observer une fois : `{"command":["observe_property", 1, "time-pos"]}`, idem `duration` et `pause`
- **Événement `end-file`** : lire le champ `reason`. `eof` → passer à la piste suivante ; `stop` (déclenché par vous) → **ne pas avancer**. Confondre les deux produit des doubles sauts, le bug classique de cette architecture.

Une tâche lectrice parse chaque ligne en `PlayerEvent` et l'envoie sur un `mpsc`. Implémenter `Drop` (et un handler SIGINT) qui tue le processus mpv enfant et supprime la socket.

Pas de gapless dans le MVP : sur `end-file{eof}`, résoudre l'URL suivante puis `loadfile` (~300 ms de blanc acceptable).

---

## 8. Boucle applicative et UI

Un seul `tokio::select!` sur trois sources : événements terminal, événements player, résultats des tâches API.

```rust
loop {
    tokio::select! {
        Some(Ok(ev)) = term_events.next() => actions.push(map_key(ev)),
        Some(ev)     = player_rx.recv()   => actions.push(Action::Player(ev)),
        Some(res)    = api_rx.recv()      => actions.push(Action::Api(res)),
    }
    for a in actions.drain(..) { cmds.extend(app.handle(a)); }
    dispatch(cmds);
    terminal.draw(|f| ui::render(f, &app))?;
}
```

`App` contient `screen`, `playlists`, `queue`, `now: Option<NowPlaying>`, `status`. `handle(Action)` est un reducer (pur autant que possible) qui renvoie des `Command` exécutées par la boucle : l'état est testable sans terminal ni réseau. Le rendu est une fonction pure de l'état.

**Écrans** : liste des playlists → liste des pistes avec pied de page now-playing.
**Touches** : `j`/`k` ou flèches, `Entrée` sélectionner, `Espace` pause, `n`/`p` suivant/précédent, `Échap` retour, `q` quitter.

`ratatui::init()` installe un hook de panic qui restaure le terminal ; appeler `ratatui::restore()` à la sortie et intercepter `tokio::signal::ctrl_c` pour ne pas laisser un terminal en mode raw avec mpv encore lancé.

Commandes non interactives « gratuites » : `rsc login`, `rsc playlists`, `rsc play "<nom-ou-id>"`.

---

## 9. Jalons

| # | Livrable | Estimation |
|---|---|---|
| 0 | Gate 0 : app enregistrée ; curl de `/me`, `/me/playlists`, une URL de stream | 1-2 h |
| 1 | `rsc login` : PKCE + loopback + fichier de tokens + `/me` affiche le username | 4-6 h |
| 2 | Refresh, écriture atomique, retry sur 401 (tester en expirant le token à la main) | 2-3 h |
| 3 | `rsc playlists` : liste paginée sur stdout | 2 h |
| 4 | Wrapper IPC mpv + `rsc play <url-de-piste>` avec pause/quit | 3-4 h |
| 5 | Queue + avance auto sur `end-file{eof}`, `n`/`p` | 2-3 h |
| 6 | Écrans ratatui à la place des commandes stdout | 3-4 h |
| 7 | Gestion d'erreurs, login `--manual`, README, `cargo-deb` | 2-3 h |

Les jalons 0 à 5 couvrent déjà les trois fonctionnalités demandées ; le 6 est le confort. Chaque jalon doit livrer un binaire fonctionnel. **Sans accès officiel, faire 1 à 6 contre le mock (§12).**

---

## 10. Tests

- **Unitaires** : challenge PKCE contre le vecteur de test de la RFC 7636 ; pagination `next_href` avec `wiremock` ; logique de queue (avance, saut des `blocked`, bornes) ; transitions de `App::handle`.
- **Intégration** : `FakePlayer` qui vérifie la séquence de `loadfile` pour une queue de 3 pistes dont une `blocked`.
- **Manuels** : token expiré, coupure réseau en cours de piste, playlist sans piste jouable, Ctrl-C pendant la lecture (vérifier `pgrep mpv` ensuite).

---

## 11. Pièges, par ordre de probabilité de vous coûter une soirée

1. Perdre le refresh token faute de l'avoir persisté atomiquement avant usage.
2. Double avance en traitant tout `end-file` comme un EOF.
3. Résoudre les URLs de stream trop tôt : elles expirent en cours de playlist.
4. `redirect_uri` non identique (slash final, `localhost` vs `127.0.0.1`, port qui change).
5. Processus mpv orphelins et terminal cassé après un panic.

---

## 12. Développer et tester sans accès officiel à l'API

Objectif : valider auth, pagination, refresh, queue, player et TUI **sans jamais toucher SoundCloud**. Trois options, cumulables ; A et B sont les plus rentables.

### Option A — Backend local (zéro réseau)

Un `LocalBackend` qui implémente `Backend` en traitant chaque **dossier** de `~/Music/rsc-dev/` comme une playlist et chaque fichier audio (trié par nom) comme une piste. Il exerce toute la chaîne queue → mpv → TUI, avec l'ordre de lecture, l'avance automatique et le `end-file`.

Générer des pistes courtes et **reconnaissables à l'oreille** (fréquence différente par piste, on entend l'ordre) :

```bash
mkdir -p ~/Music/rsc-dev/test-playlist && cd ~/Music/rsc-dev/test-playlist
for i in 1 2 3 4 5; do
  ffmpeg -f lavfi -i "sine=frequency=$((330 + i*110)):duration=8" \
         -c:a libmp3lame -q:a 4 "0$i-tone-$i.mp3"
done
```

Sélection : `rsc --backend local` (ou `RSC_BACKEND=local`).

### Option B — Serveur mock `rsc-mock` (teste aussi l'OAuth)

Petit binaire `axum` qui imite le contrat de l'API ; l'app le vise via `RSC_API_BASE=http://127.0.0.1:9000` et `RSC_AUTH_BASE=http://127.0.0.1:9000` (fausses `client_id`/`client_secret`). C'est la seule option qui teste réellement le login et le refresh.

| Route | Comportement |
|---|---|
| `GET /authorize` | Mémorise `code_challenge`/`state`, répond `302` vers `redirect_uri?code=…&state=…` (le vrai flux navigateur → loopback est donc exercé) |
| `POST /oauth/token` | `authorization_code` : **vérifie `sha256(code_verifier)` contre le challenge mémorisé** (détecte les bugs PKCE). `refresh_token` : **usage unique**, un rejeu renvoie `400 invalid_grant`. `expires_in` réglable (`--expires-in 20`) pour provoquer des refresh |
| `GET /me` | `401` si token absent/expiré |
| `GET /me/playlists` | Pagination avec `collection` + `next_href` (par ex. 3 par page pour forcer plusieurs pages) |
| `GET /playlists/{id}` | Pistes en ordre fixe ; option pour renvoyer une liste **tronquée** (IDs seuls) |
| `GET /tracks?ids=…` | Hydratation par lots |
| Route de stream | Renvoie une URL `http://127.0.0.1:9000/media/xx.mp3` servie par `tower_http::services::ServeDir` depuis un dossier de fixtures (mêmes MP3 que l'option A) |

Jeu de données conseillé : une playlist de 8 pistes avec 1 `blocked` et 1 `preview`, une playlist vide, une playlist de 120 pistes (pagination).

Flags d'injection de pannes : `--expires-in <s>`, `--fail-next 401|429|503`, `--truncate-tracks`, `--slow-token-ms <n>`, `--reject-refresh-once`.

Scénarios à jouer systématiquement :

| Scénario | Ce qu'on vérifie |
|---|---|
| Login complet | PKCE, `state`, échange du code, écriture `0600` |
| `--expires-in 20` + lecture longue | Refresh silencieux, écriture atomique du nouveau refresh token |
| Deux requêtes concurrentes sur token expiré | Un seul refresh (mutex + revérification) |
| Rejeu d'un refresh token | Erreur propre → retour au login |
| `--fail-next 401` | Un refresh + un retry, pas de boucle |
| `--fail-next 429` / `503` | Backoff, message dans l'UI |
| Playlist avec `blocked` / `preview` | Saut et marquage dans l'UI |
| Playlist tronquée | Chemin d'hydratation par lots |
| Mauvais `state` sur le callback | Rejet |
| Port 8888 déjà occupé | Message d'erreur clair |

### Option C — Lecture réelle via `yt-dlp` + `mpv` (zone grise)

Pour tester seulement le **player et la TUI sur du vrai contenu SoundCloud** : un `YtDlpBackend` qui lance `yt-dlp -J --flat-playlist <url-de-playlist-publique>` et passe les URLs à mpv. Limites à connaître :

- Ne teste **ni OAuth ni l'API officielle** ; les playlists privées nécessitent des cookies de navigateur.
- C'est un accès non officiel : à réserver à des essais personnels et **à ne pas livrer** dans le projet public.
- Sert aussi de repli si le Gate 0 échoue.

### Compléments utiles

- **Spec OpenAPI** (`openapi/api.yaml` du dépôt `soundcloud/api`) : s'en servir pour valider vos modèles serde et vos fixtures. Un outil comme Prism (`prism mock api.yaml`) génère des réponses génériques mais **ne gère ni PKCE, ni refresh à usage unique, ni audio** : il ne remplace pas `rsc-mock`.
- **Enregistrer des fixtures réelles** dès que l'accès est disponible (réponses de `/me/playlists`, `/playlists/{id}`, stream), les anonymiser et les rejouer dans `wiremock` pour des tests de contrat.

### Ce que le mock ne peut PAS valider

À vérifier impérativement sur la vraie API avant toute release :

1. La forme exacte de `GET /playlists/{id}` sur une grosse playlist (troncature ou non).
2. Le bon endpoint de stream (`stream_url` vs `/stream` vs `/streams`) et la structure de sa réponse (progressif vs HLS).
3. Si l'URL de stream exige l'en-tête `Authorization` côté mpv.
4. Le format réel des erreurs (`401`, `403`, `429`) et leur corps JSON.
5. Le comportement exact de la correspondance `redirect_uri` et des scopes.
6. La proportion de pistes `blocked`/`preview` dans vos vraies playlists.

### Ordre de travail recommandé

1. `LocalBackend` + `Player` mpv + queue → lecture d'une playlist dans l'ordre (jalons 4-5).
2. `rsc-mock` avec OAuth complet → jalons 1-3 et tests de refresh.
3. TUI (jalon 6) sur le mock.
4. Dès l'accès officiel : Gate 0, enregistrement de fixtures, correction des écarts listés ci-dessus, puis bascule des URLs de base par défaut.
