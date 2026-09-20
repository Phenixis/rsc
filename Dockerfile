# Image de développement de rsc : toolchain Rust + mpv + ffmpeg.
# Le code source n'est PAS copié dans l'image : il est monté en volume (voir compose.yaml).
FROM rust:1-trixie

# mpv      : lecteur piloté par IPC (dépendance runtime du projet)
# ffmpeg   : génération des pistes de test (plan §12, option A)
# procps   : pgrep, pour vérifier qu'aucun mpv orphelin ne survit (plan §10)
# jq       : lire les réponses JSON de l'API pendant le Gate 0
RUN apt-get update \
    && apt-get install -y --no-install-recommends mpv ffmpeg procps jq \
    && rm -rf /var/lib/apt/lists/*

RUN rustup component add clippy rustfmt

# Utilisateur non-root avec le même UID/GID que l'hôte, sinon target/ et
# Cargo.lock appartiendraient à root dans le dépôt.
ARG UID=1000
ARG GID=1000
RUN groupadd --gid "$GID" dev \
    && useradd --create-home --uid "$UID" --gid "$GID" --shell /bin/bash dev \
    && mkdir -p /workspace /home/dev/.cargo \
    && chown dev:dev /workspace /home/dev/.cargo

# Le toolchain reste dans /usr/local/rustup (lecture seule) ; le cache du registre
# et les outils installés par `cargo install` vont dans le home de l'utilisateur.
ENV CARGO_HOME=/home/dev/.cargo
ENV PATH=/home/dev/.cargo/bin:/usr/local/cargo/bin:$PATH

USER dev
WORKDIR /workspace
