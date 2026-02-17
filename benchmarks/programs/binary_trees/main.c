#include <stdio.h>
#include <stdlib.h>

typedef struct Node { struct Node *left, *right; } Node;

Node *make(int depth) {
    Node *n = malloc(sizeof(Node));
    if (depth > 0) {
        n->left = make(depth - 1);
        n->right = make(depth - 1);
    } else {
        n->left = n->right = NULL;
    }
    return n;
}

long check(Node *n) {
    if (!n->left) return 1;
    return 1 + check(n->left) + check(n->right);
}

void free_tree(Node *n) {
    if (n->left) { free_tree(n->left); free_tree(n->right); }
    free(n);
}

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    int n = atoi(argv[1]);
    int min_depth = 4;
    int max_depth = n;
    if (min_depth + 2 > max_depth) max_depth = min_depth + 2;

    Node *stretch = make(max_depth + 1);
    printf("stretch tree of depth %d\t check: %ld\n", max_depth + 1, check(stretch));
    free_tree(stretch);

    Node *long_lived = make(max_depth);

    for (int depth = min_depth; depth <= max_depth; depth += 2) {
        int iterations = 1 << (max_depth - depth + min_depth);
        long total_check = 0;
        for (int i = 0; i < iterations; i++) {
            Node *t = make(depth);
            total_check += check(t);
            free_tree(t);
        }
        printf("%d\t trees of depth %d\t check: %ld\n", iterations, depth, total_check);
    }
    printf("long lived tree of depth %d\t check: %ld\n", max_depth, check(long_lived));
    free_tree(long_lived);
    return 0;
}
