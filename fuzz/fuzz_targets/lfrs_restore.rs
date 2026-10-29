#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    laneflow_fuzz::check_snapshot(data);
});
