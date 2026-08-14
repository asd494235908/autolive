FROM --platform=$BUILDPLATFORM docker.1ms.run/library/golang:1.22-alpine AS build

WORKDIR /src
COPY go.mod go.sum ./
RUN go mod download
COPY . .

ARG TARGETOS
ARG TARGETARCH
RUN CGO_ENABLED=0 GOOS=${TARGETOS} GOARCH=${TARGETARCH} \
    go build -trimpath -ldflags='-s -w' -o /out/autolive-api ./cmd/server && \
    CGO_ENABLED=0 GOOS=${TARGETOS} GOARCH=${TARGETARCH} \
    go build -trimpath -ldflags='-s -w' -o /out/autolive-migrate ./cmd/migrate

FROM docker.1ms.run/library/alpine:3.20

RUN apk add --no-cache ca-certificates wget

COPY --from=build /out/autolive-api /usr/local/bin/autolive-api
COPY --from=build /out/autolive-migrate /usr/local/bin/autolive-migrate

USER 65532:65532
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=5 \
  CMD wget -q -O - http://127.0.0.1:8080/api/v1/health >/dev/null || exit 1
CMD ["/usr/local/bin/autolive-api"]
