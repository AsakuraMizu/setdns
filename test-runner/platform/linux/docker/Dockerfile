FROM rust:1-bookworm

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        dbus \
        iproute2 \
        systemd-resolved \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /repo
