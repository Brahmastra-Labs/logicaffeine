import sys, json
reps = int(sys.argv[1])
tail = "".join("," + str(k) for k in range(1, 500+1)) + "]"
checksum = 0
for r in range(reps):
    js = "[" + str(r % 1000) + tail
    checksum += len(json.dumps(json.loads(js), separators=(",", ":")))
print(checksum)
