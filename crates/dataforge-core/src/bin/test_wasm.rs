fn main() {
    let mut config = wasmi::Config::default();
    config.consume_fuel(true);
    let engine = wasmi::Engine::new(&config);
    let mut store = wasmi::Store::new(&engine, ());
    
    // We want to test compiling these:
    let _ = store.add_fuel(10);
    let _ = store.fuel_consumed();
}
