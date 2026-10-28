---
sidebar_position: 3
---

# Speed Up Builds With Your Own Dockerfile

If your build fails with `Build timeout after ... seconds` while it is downloading packages, for example

```
Downloading django-5.2.18-py3-none-any.whl (8.3 MB)
Build timeout after 600 seconds
```

the download from PyPI or the Alpine package CDN was too slow. PWS keeps a local cache of both, so a package someone has already installed downloads in seconds.

Projects **without** a `Dockerfile` use the cache automatically. If your project has its own `Dockerfile`, you need to opt in.

## Using the Cache

Add these `ARG` lines to your `Dockerfile`, in every stage that installs packages and before the `RUN` lines that install them:

```dockerfile
FROM python:3.13-alpine

# PWS package cache. Left empty outside PWS, so the Dockerfile still builds
# anywhere else.
ARG PIP_INDEX_URL
ARG PIP_TRUSTED_HOST
ARG UV_DEFAULT_INDEX
ARG UV_INSECURE_HOST
ARG APK_MIRROR

RUN if [ -n "$APK_MIRROR" ]; then \
        sed -i -E "s#^https?://dl-cdn.alpinelinux.org/alpine#$APK_MIRROR#" /etc/apk/repositories; \
    fi \
    && apk add --no-cache gcc musl-dev

COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt
```

`pip` and `uv` read these variables on their own, so the install commands stay the same. Keep only the lines you need: the `APK_MIRROR` part only matters for Alpine-based images that run `apk add`.

PWS only passes the variables your `Dockerfile` declares. If the cache is unavailable it passes nothing, and your build downloads from PyPI as usual.

## Using Your Own Package Index

If you set `PIP_INDEX_URL` (or any of the variables above) as an environment variable of your project, PWS keeps your value and does not replace it with the cache.
