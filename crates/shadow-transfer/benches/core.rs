use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use shadow_protocol::{decode_control, encode_control, FileChunk, Message, Ping, PROTOCOL_VERSION};

fn hashing(c: &mut Criterion) {
    let data = vec![0x5Au8; 4 * 1024 * 1024];
    let mut group = c.benchmark_group("blake3");
    group.throughput(Throughput::Bytes(data.len() as u64));
    group.bench_function("hash_4_mib", |b| {
        b.iter(|| blake3::hash(black_box(&data)));
    });
    group.finish();
}

fn framing(c: &mut Criterion) {
    let message = Message::FileChunk(FileChunk {
        file_id: 1,
        offset: 1024 * 1024,
        len: 1024 * 1024,
        hash: "ab".repeat(32),
    });
    let encoded = encode_control(&message).unwrap();
    c.bench_function("decode_chunk_control", |b| {
        b.iter(|| decode_control(black_box(&encoded)).unwrap());
    });

    let hello = Message::Hello(shadow_protocol::Hello {
        protocol: PROTOCOL_VERSION,
        device_name: "Benchmark device".into(),
        nonce: "01".repeat(16),
    });
    let encoded = encode_control(&hello).unwrap();
    c.bench_function("decode_hello_control", |b| {
        b.iter(|| decode_control(black_box(&encoded)).unwrap());
    });

    let ping = Message::Ping(Ping {
        nonce: "02".repeat(16),
    });
    c.bench_function("encode_ping", |b| {
        b.iter(|| encode_control(black_box(&ping)).unwrap());
    });
}

criterion_group!(benches, hashing, framing);
criterion_main!(benches);
