# syntax=docker/dockerfile:1
# Four targets share the same task, Python, Pi, and native utilities.
# SAT is mounted at runtime, never compiled or copied into these images.
FROM ghcr.io/astral-sh/uv:0.9.11 AS uv
FROM node:24-trixie-slim AS common
ARG BENCH_UID=1000
ARG BENCH_GID=1000
COPY --from=uv /uv /usr/local/bin/uv
RUN apt-get update && apt-get install -y --no-install-recommends \
    bash ca-certificates curl git ripgrep procps python3 \
    && rm -rf /var/lib/apt/lists/*
ENV UV_PYTHON_INSTALL_DIR=/opt/python UV_CACHE_DIR=/tmp/uv-cache
RUN uv python install 3.9.20 && uv venv --python 3.9.20 /opt/task-env
ENV PATH=/opt/task-env/bin:/opt/benchmark/node_modules/.bin:/usr/local/bin:/usr/bin:/bin
RUN git init /testbed && cd /testbed \
    && git remote add origin https://github.com/psf/requests.git \
    && git fetch --depth 1 origin 091991be0da19de9108dbe5e3752917fea3d7fdc \
    && git checkout --detach FETCH_HEAD && git remote remove origin \
    && git config user.name Benchmark && git config user.email benchmark@localhost \
    && uv pip install --python /opt/task-env/bin/python pytest==7.4.4 -e /testbed
WORKDIR /opt/benchmark
COPY package.json package-lock.json ./
RUN npm ci --ignore-scripts && npm cache clean --force
COPY agent-extension.ts container-run.py reporting.py tool_config.py task_prompts.py ./
ENV HOME=/tmp/pi-home PI_CODING_AGENT_DIR=/tmp/pi-agent PI_OFFLINE=1 RTK_TELEMETRY_DISABLED=1 SAT_HOME=/sat-registry
RUN mkdir -p /tmp/pi-agent /tmp/pi-home /sat-registry /results \
    && chown -R ${BENCH_UID}:${BENCH_GID} /testbed /tmp/pi-agent /tmp/pi-home /sat-registry
WORKDIR /testbed
ENTRYPOINT ["/usr/bin/python3", "/opt/benchmark/container-run.py"]

FROM common AS baseline
ENV BENCH_VARIANT=baseline

FROM common AS sat
ENV BENCH_VARIANT=sat

FROM common AS with_rtk
ARG RTK_VERSION=1.2.0
ARG RTK_SHA256
# This benchmark initially targets linux/amd64, like the selected SWE-bench image.
RUN test -n "$RTK_SHA256" \
    && curl -fL --retry 3 -o /tmp/rtk.tar.gz \
       "https://github.com/rtk-ai/rtk/releases/download/v${RTK_VERSION}/rtk-x86_64-unknown-linux-musl.tar.gz" \
    && echo "$RTK_SHA256  /tmp/rtk.tar.gz" | sha256sum --check - \
    && mkdir /tmp/rtk-extract && tar -xzf /tmp/rtk.tar.gz -C /tmp/rtk-extract \
    && install "$(find /tmp/rtk-extract -type f -name rtk -print -quit)" /usr/local/bin/rtk \
    && rm -rf /tmp/rtk.tar.gz /tmp/rtk-extract

FROM with_rtk AS rtk
ENV BENCH_VARIANT=rtk

FROM with_rtk AS sat_rtk
ENV BENCH_VARIANT=sat_rtk
