FROM --platform=$BUILDPLATFORM rust:1.69 as build_stage

WORKDIR /build

COPY ./src ./src
COPY ./Cargo.toml ./

RUN cargo fetch --verbose && \
    cargo build --verbose --offline --release

FROM --platform=$BUILDPLATFORM debian:bullseye-slim as final_image
RUN apt-get update \
    && apt-get install ca-certificates -y
COPY \
    --from=build_stage \
    /build/target/release/notification_bot /usr/bin/

WORKDIR /app
VOLUME [ "/app" ]

CMD ["notification_bot"]