# JPEG Carver Specification

Detect SOI markers, parse marker segments with strict length bounds, identify valid scan structure and seek EOI while enforcing maximum candidate size. Validate output with an independent decoder/parser where feasible. Fragmentation without filesystem extents is explicitly uncertain.