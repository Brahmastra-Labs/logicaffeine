CURRICULUM="$(cd "$(dirname "$0")/../.." && pwd)/apps/logicaffeine_web/assets/curriculum"
OUTPUT="curriculum_bundle.txt"
> "$OUTPUT"

for era in "$CURRICULUM"/*/; do
    era=$(basename "$era")
    echo "=== ERA: $era ===" >> "$OUTPUT"
    cat "$CURRICULUM/$era/meta.json" >> "$OUTPUT"
    echo "" >> "$OUTPUT"

    for module in "$CURRICULUM/$era"/*/; do
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