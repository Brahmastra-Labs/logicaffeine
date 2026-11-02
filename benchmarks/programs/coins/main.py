import sys


def main():
    n = int(sys.argv[1])
    coins = [1, 5, 10, 25, 50, 100]
    dp = [0] * (n + 1)
    dp[0] = 1
    for c in coins:
        for j in range(c, n + 1): dp[j] = (dp[j] + dp[j - c]) % 1000000007
    checksum = 0
    for v in dp: checksum = (checksum * 31 + v) & 268435455
    print(dp[n], checksum)


if __name__ == "__main__":
    main()
