@build:
    cargo build
    cp -R proto_ts/* ../sharify/src/lib/proto

@run:
    cargo run

@test:
    cargo test -- --nocapture

@clean_proto:
    find src/proto -maxdepth 1 -type f -name '*.rs' ! -name 'mod.rs' -exec rm {} \+
    rm -rf proto_ts/*

@clean: clean_proto
    cargo clean
