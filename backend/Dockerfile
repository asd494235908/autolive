# golang:1.26.6-alpine (Docker Hub manifest digest)
FROM --platform=$BUILDPLATFORM docker.1ms.run/library/golang@sha256:3889b425f035be855a72fb4755265311293b6d414521f0a519d819df32222d83 AS build

WORKDIR /src
COPY go.mod go.sum ./
RUN go mod download
COPY . .

ARG TARGETOS
ARG TARGETARCH
RUN CGO_ENABLED=0 GOOS=${TARGETOS} GOARCH=${TARGETARCH} \
    go build -trimpath -ldflags='-s -w' -o /out/autolive-api ./cmd/server && \
    CGO_ENABLED=0 GOOS=${TARGETOS} GOARCH=${TARGETARCH} \
    go build -trimpath -ldflags='-s -w' -o /out/autolive-migrate ./cmd/migrate && \
    CGO_ENABLED=0 GOOS=${TARGETOS} GOARCH=${TARGETARCH} \
    go build -trimpath -ldflags='-s -w' -o /out/autolive-backfill-normalized ./cmd/backfill-normalized && \
    CGO_ENABLED=0 GOOS=${TARGETOS} GOARCH=${TARGETARCH} \
    go build -trimpath -ldflags='-s -w' -o /out/autolive-verify-normalized-cutover ./cmd/verify-normalized-cutover

# alpine:3.20 (Docker Hub manifest digest)
FROM docker.1ms.run/library/alpine@sha256:d9e853e87e55526f6b2917df91a2115c36dd7c696a35be12163d44e6e2a4b6bc

RUN apk add --no-cache ca-certificates wget

COPY --from=build /out/autolive-api /usr/local/bin/autolive-api
COPY --from=build /out/autolive-migrate /usr/local/bin/autolive-migrate
COPY --from=build /out/autolive-backfill-normalized /usr/local/bin/autolive-backfill-normalized
COPY --from=build /out/autolive-verify-normalized-cutover /usr/local/bin/autolive-verify-normalized-cutover

USER 65532:65532
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=5 \
  CMD wget -q -O - http://127.0.0.1:8080/api/v1/readyz >/dev/null || exit 1
CMD ["/usr/local/bin/autolive-api"]
