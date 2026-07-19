// =============================================================================
// DataForge Node.js Example — Streaming CSV Read
// =============================================================================
// Demonstrates how to stream CSV files with minimal memory footprint.
// =============================================================================

import { writeFileSync, unlinkSync } from 'fs';
import { createRequire } from 'module';
const require = createRequire(import.meta.url);

// Import the native module
const dataforge = require('../../crates/dataforge-node/index.js');

// Create a dummy CSV file
const csvContent = `name,age,city
Alice,30,New York
Bob,25,Los Angeles
Charlie,35,San Francisco
Diana,28,Miami`;
writeFileSync('demo.csv', csvContent);

console.log('Loading CSV using native engine...');

// Open CSV reader with batch size of 2
const reader = dataforge.JsCsvReader.open('demo.csv', 2, false);

console.log('Headers:', reader.headers);

let batch;
let batchCount = 0;

while ((batch = reader.nextBatch()) !== null) {
  batchCount++;
  console.log(`\n--- Batch ${batchCount} (${batch.rowCount} rows) ---`);
  
  // Convert rows to JS JSON objects
  const objects = batch.toJsonObjects();
  console.log(objects);
}

// Clean up
unlinkSync('demo.csv');
console.log('\nDemo complete!');
