fn main() {
    let mut config = wasmi::Config::default();
    config.consume_fuel(true);
    let engine = wasmi::Engine::new(&config);
    let mut store = wasmi::Store::new(&engine, ());
    
    // We want to test compiling these:
    let _ = store.add_fuel(10);
    
    // Uncomment these one by one or test all to see what is missing:
    // let _ = store.get_fuel(); // We know this fails
    let _ = store.fuel_consumed();
    let _ = store.reset_fuel();
}
