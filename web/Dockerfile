# syntax=docker/dockerfile:1
# Build from the repository root so the runtime-owned Caddyfile is available.
FROM node:24-bookworm-slim AS build
WORKDIR /app
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

FROM caddy:2-alpine AS runtime
# Explicit probe dependency; do not rely on utilities in the base image.
RUN apk add --no-cache curl
COPY deploy/Caddyfile /etc/caddy/Caddyfile
COPY --from=build /app/dist /srv
EXPOSE 80
