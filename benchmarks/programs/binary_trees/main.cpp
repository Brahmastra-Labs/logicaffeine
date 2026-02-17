#include <cstdio>
#include <cstdlib>
struct Node { Node *l, *r; };
Node *make(int d) {
    Node *n = new Node;
    if (d > 0) { n->l = make(d-1); n->r = make(d-1); }
    else { n->l = n->r = nullptr; }
    return n;
}
long check(Node *n) { return n->l ? 1+check(n->l)+check(n->r) : 1; }
void del(Node *n) { if (n->l) { del(n->l); del(n->r); } delete n; }
int main(int argc, char *argv[]) {
    if (argc<2) return 1;
    int n=atoi(argv[1]), mn=4, mx=n; if(mn+2>mx) mx=mn+2;
    Node *s=make(mx+1); printf("stretch tree of depth %d\t check: %ld\n",mx+1,check(s)); del(s);
    Node *ll=make(mx);
    for(int d=mn;d<=mx;d+=2) {
        int it=1<<(mx-d+mn); long tc=0;
        for(int i=0;i<it;i++){Node*t=make(d);tc+=check(t);del(t);}
        printf("%d\t trees of depth %d\t check: %ld\n",it,d,tc);
    }
    printf("long lived tree of depth %d\t check: %ld\n",mx,check(ll)); del(ll);
    return 0;
}
