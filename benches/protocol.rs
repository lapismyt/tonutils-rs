use criterion::{Criterion, black_box, criterion_group, criterion_main};
use std::sync::Arc;
use std::time::Duration;
use tl_proto::TlRead;
use tokio_util::bytes::{Bytes, BytesMut};
use tokio_util::codec::{Decoder, Encoder};
use tonutils::adnl::{AdnlAesParams, AdnlCodec};
use tonutils::tl::request::{GetMasterchainInfoExt, Request};
use tonutils::tvm::{
    Builder, Cell, Slice, TvmStack, TvmStackEntry, deserialize_boc, serialize_boc,
};

fn fixture_cell() -> Arc<Cell> {
    let mut child = Builder::new();
    child.store_u32(0x746f_6e75).unwrap();
    child.store_bytes(b"protocol benchmark child").unwrap();
    let child = child.build().unwrap();

    let mut root = Builder::new();
    root.store_u64(0x1122_3344_5566_7788).unwrap();
    root.store_bytes(b"protocol benchmark root").unwrap();
    root.store_ref(child).unwrap();
    root.build().unwrap()
}

fn fixture_stack(cell: Arc<Cell>) -> TvmStack {
    TvmStack::new(vec![
        TvmStackEntry::int(1_234_567_890_i64),
        TvmStackEntry::Cell(cell.clone()),
        TvmStackEntry::Tuple(vec![
            TvmStackEntry::Null,
            TvmStackEntry::Slice(cell),
            TvmStackEntry::Unsupported(vec![1, 2, 3, 4]),
        ]),
    ])
}

fn bench_adnl_codec(c: &mut Criterion) {
    let mut group = c.benchmark_group("adnl_codec");
    group.sample_size(20);

    let params = AdnlAesParams::default();
    let payload = Bytes::from(vec![0x55; 512]);

    group.bench_function("encode_512_bytes", |b| {
        b.iter(|| {
            let mut codec = AdnlCodec::client(&params);
            let mut frame = BytesMut::new();
            codec
                .encode(black_box(payload.clone()), &mut frame)
                .unwrap();
            black_box(frame)
        })
    });

    group.bench_function("encode_decode_512_bytes", |b| {
        b.iter(|| {
            let mut encoder = AdnlCodec::client(&params);
            let mut decoder = AdnlCodec::server(&params);
            let mut frame = BytesMut::new();
            encoder
                .encode(black_box(payload.clone()), &mut frame)
                .unwrap();
            black_box(decoder.decode(&mut frame).unwrap().unwrap())
        })
    });

    group.finish();
}

fn bench_tl_codec(c: &mut Criterion) {
    let mut group = c.benchmark_group("tl_codec");
    let request = Request::GetMasterchainInfoExt(GetMasterchainInfoExt { mode: 7 });
    let encoded = tl_proto::serialize(request.clone());

    group.bench_function("serialize_get_masterchain_info_ext", |b| {
        b.iter(|| black_box(tl_proto::serialize(black_box(request.clone()))))
    });

    group.bench_function("deserialize_get_masterchain_info_ext", |b| {
        b.iter(|| {
            black_box(
                Request::read_from(&mut encoded.as_slice())
                    .expect("fixture request should deserialize"),
            )
        })
    });

    group.finish();
}

fn bench_tvm_cells_and_boc(c: &mut Criterion) {
    let mut group = c.benchmark_group("tvm_cell_boc");
    let cell = fixture_cell();
    let boc = serialize_boc(&cell, false).unwrap();

    group.bench_function("cell_hash_with_ref", |b| b.iter(|| black_box(cell.hash())));

    group.bench_function("boc_serialize_with_ref", |b| {
        b.iter(|| black_box(serialize_boc(black_box(&cell), false).unwrap()))
    });

    group.bench_function("boc_deserialize_with_ref", |b| {
        b.iter(|| black_box(deserialize_boc(black_box(&boc)).unwrap()))
    });

    group.bench_function("builder_1023_bit_pattern", |b| {
        let bits = [0xa5; 128];
        b.iter(|| {
            let mut builder = Builder::new();
            builder.store_bits(black_box(&bits), 1023).unwrap();
            black_box(builder.build().unwrap())
        })
    });

    group.bench_function("slice_read_1023_bit_pattern", |b| {
        let mut builder = Builder::new();
        builder.store_bits(&[0xa5; 128], 1023).unwrap();
        let cell = builder.build().unwrap();
        b.iter(|| {
            let mut slice = Slice::new(cell.clone());
            black_box(slice.load_bits(1023).unwrap())
        })
    });

    group.finish();
}

fn bench_tvm_stack(c: &mut Criterion) {
    let mut group = c.benchmark_group("tvm_stack");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(10));

    let stack = fixture_stack(fixture_cell());
    let boc = stack.to_boc().unwrap();

    group.bench_function("stack_to_boc_nested", |b| {
        b.iter(|| black_box(stack.to_boc().unwrap()))
    });

    group.bench_function("stack_from_boc_nested", |b| {
        b.iter(|| black_box(TvmStack::from_boc(black_box(&boc)).unwrap()))
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_adnl_codec,
    bench_tl_codec,
    bench_tvm_cells_and_boc,
    bench_tvm_stack
);
criterion_main!(benches);
