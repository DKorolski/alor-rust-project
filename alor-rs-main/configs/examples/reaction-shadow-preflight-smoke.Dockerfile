# Owned tests only: no broker secrets, service Redis or writable production mount.
FROM debian:12-slim@sha256:7b140f374b289a7c2befc338f42ebe6441b7ea838a042bbd5acbfca6ec875818
RUN apt-get update && apt-get install -y --no-install-recommends python3 redis-server redis-tools ca-certificates && rm -rf /var/lib/apt/lists/*
ENV PYTHONDONTWRITEBYTECODE=1
ENTRYPOINT ["/usr/bin/python3"]
