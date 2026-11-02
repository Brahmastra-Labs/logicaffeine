import sys
def lwrap(s, width):
    out = []
    cur = ""
    for w in s.split():
        if len(cur) == 0:
            cur = w
        else:
            cand = cur + " " + w
            if len(cand) <= width:
                cur = cand
            else:
                out.append(cur)
                cur = w
    if len(cur) > 0:
        out.append(cur)
    return out
reps = int(sys.argv[1])
base = "abcABC012 " * 400
checksum = 0
for r in range(reps):
    s = "a" * (r % 100) + base
    checksum += len("\n".join(lwrap(s, 40)))
print(checksum)
