# =============================================================================
# DataForge Python Example — Streaming CSV Read
# =============================================================================
# Demonstrates how to stream CSV files in Python with zero memory overhead
# and high-performance native parsing.
# =============================================================================

import os

# Try to import the local compiled module.
# You need to run `maturin develop` inside crates/dataforge-python/ first.
try:
    import dataforge
except ImportError:
    print("Error: Could not import dataforge. Please build it first using:")
    print("  cd crates/dataforge-python && maturin develop")
    exit(1)

# Write a dummy CSV file
csv_content = """name,age,city
Alice,30,New York
Bob,25,Los Angeles
Charlie,35,San Francisco
Diana,28,Miami"""

with open("demo.csv", "w") as f:
    f.write(csv_content)

print("Streaming CSV with batch size of 2...")

# Open the reader
reader = dataforge.PyCsvReader("demo.csv", batch_size=2)

print("Headers:", reader.headers)

# Iterate over batches
for idx, batch in enumerate(reader):
    print(f"\n--- Batch {idx + 1} ({batch.row_count} rows) ---")
    
    # Convert batch to list of dicts (Pandas compatible)
    dicts = batch.to_dicts()
    print(dicts)

# Clean up
os.remove("demo.csv")
print("\nDemo complete!")
