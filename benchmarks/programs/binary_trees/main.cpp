#include <cstdio>
#include <cstdlib>

struct Node {
    Node *left, *right;
};

Node *bottom_up_tree(int d) {
    Node *left = nullptr, *right = nullptr;
    if (d > 0) {
        left = bottom_up_tree(d - 1);
        right = bottom_up_tree(d - 1);
    }
    return new Node{left, right};
}

long check(const Node *node) {
    if (!node->left) return 1;
    return 1 + check(node->left) + check(node->right);
}

void delete_tree(Node *node) {
    if (node->left) {
        delete_tree(node->left);
        delete_tree(node->right);
    }
    delete node;
}

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    int n = atoi(argv[1]), mn = 4, mx = n;
    if (mn + 2 > mx) mx = mn + 2;
    Node *stretch = bottom_up_tree(mx + 1);
    printf("stretch tree of depth %d\t check: %ld\n", mx + 1, check(stretch));
    delete_tree(stretch);
    Node *ll = bottom_up_tree(mx);
    for (int d = mn; d <= mx; d += 2) {
        int it = 1 << (mx - d + mn);
        long tc = 0;
        for (int i = 0; i < it; i++) {
            Node *tree = bottom_up_tree(d);
            tc += check(tree);
            delete_tree(tree);
        }
        printf("%d\t trees of depth %d\t check: %ld\n", it, d, tc);
    }
    printf("long lived tree of depth %d\t check: %ld\n", mx, check(ll));
    delete_tree(ll);
    return 0;
}
