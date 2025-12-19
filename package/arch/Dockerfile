FROM debian:trixie-slim

RUN apt-get update \
  && apt-get install -y --no-install-recommends \
    ca-certificates \
    coreutils \
    sed \
  && rm -rf /var/lib/apt/lists/*

COPY package/arch/build.sh /usr/local/bin/build
RUN chmod +x /usr/local/bin/build

ENTRYPOINT ["/usr/local/bin/build"]
