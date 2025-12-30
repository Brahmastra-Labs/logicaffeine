OUTPUT="curriculum_bundle.txt"
> "$OUTPUT"

for era in 00_logicaffeine 01_trivium 02_quadrivium 03_metaphysics; do
    echo "=== ERA: $era ===" >> "$OUTPUT"
    cat "assets/curriculum/$era/meta.json" >> "$OUTPUT"
    echo "" >> "$OUTPUT"

    for module in assets/curriculum/$era/*/; do
        if [ -d "$module" ]; then
            echo "--- MODULE: $(basename $module) ---" >> "$OUTPUT"
            cat "$module/meta.json" >> "$OUTPUT"
            echo "" >> "$OUTPUT"

            for exercise in "$module"/*.json; do
                if [ -f "$exercise" ] && [ "$(basename $exercise)" != "meta.json" ]; then
                    cat "$exercise" >> "$OUTPUT"
                    echo "" >> "$OUTPUT"
                fi
            done
        fi
    done
done