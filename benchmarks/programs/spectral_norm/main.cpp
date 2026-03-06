#include <cstdio>
#include <cstdlib>
#include <cmath>
double A(int i, int j) { return 1.0/((double)((i+j)*(i+j+1)/2+i+1)); }
void mulAv(int n, double *v, double *r) { for (int i=0;i<n;i++){r[i]=0;for(int j=0;j<n;j++)r[i]+=A(i,j)*v[j];} }
void mulAtv(int n, double *v, double *r) { for (int i=0;i<n;i++){r[i]=0;for(int j=0;j<n;j++)r[i]+=A(j,i)*v[j];} }
void mulAtAv(int n, double *v, double *r, double *t) { mulAv(n,v,t); mulAtv(n,t,r); }
int main(int argc, char *argv[]) {
    if (argc<2) return 1;
    int n=atoi(argv[1]);
    double *u=new double[n],*v=new double[n],*t=new double[n];
    for(int i=0;i<n;i++) u[i]=1;
    for(int i=0;i<10;i++){mulAtAv(n,u,v,t);mulAtAv(n,v,u,t);}
    double vBv=0,vv=0;
    for(int i=0;i<n;i++){vBv+=u[i]*v[i];vv+=v[i]*v[i];}
    printf("%.9f\n",sqrt(vBv/vv));
    delete[]u;delete[]v;delete[]t;
    return 0;
}
