#include <stdio.h>
#include <stdlib.h>

typedef struct Node {
    struct Node *left;
    struct Node *right;
} Node;

Node *bottom_up_tree(int depth) {
    Node *left = NULL, *right = NULL;
    if (depth > 0) {
        left = bottom_up_tree(depth - 1);
        right = bottom_up_tree(depth - 1);
    }
    Node *node = malloc(sizeof(Node));
    node->left = left;
    node->right = right;
    return node;
}

long check(const Node *node) {
    if (node->left == NULL) return 1;
    return 1 + check(node->left) + check(node->right);
}

void delete_tree(Node *node) {
    if (node->left != NULL) {
        delete_tree(node->left);
        delete_tree(node->right);
    }
    free(node);
}

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    int n = atoi(argv[1]);
    int min_depth = 4;
    int max_depth = n;
    if (min_depth + 2 > max_depth) max_depth = min_depth + 2;

    Node *stretch = bottom_up_tree(max_depth + 1);
    printf("stretch tree of depth %d\t check: %ld\n", max_depth + 1, check(stretch));
    delete_tree(stretch);

    Node *long_lived = bottom_up_tree(max_depth);

    for (int depth = min_depth; depth <= max_depth; depth += 2) {
        int iterations = 1 << (max_depth - depth + min_depth);
        long total_check = 0;
        for (int i = 0; i < iterations; i++) {
            Node *tree = bottom_up_tree(depth);
            total_check += check(tree);
            delete_tree(tree);
        }
        printf("%d\t trees of depth %d\t check: %ld\n", iterations, depth, total_check);
    }
    printf("long lived tree of depth %d\t check: %ld\n", max_depth, check(long_lived));
    delete_tree(long_lived);
    return 0;
}
