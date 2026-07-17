// Loader for native binary built by napi-rs.
try {
  module.exports = require('./dataforge.node');
} catch (e) {
  try {
    // Fallback for different build setups or environments
    module.exports = require('./build/Release/dataforge.node');
  } catch (err) {
    throw new Error('DataForge native binary not found. Please run `npm run build` or `npm run build:debug` first.');
  }
}
