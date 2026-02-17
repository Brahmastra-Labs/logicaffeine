public class Main {
    static double A(int i, int j) { return 1.0/((i+j)*(i+j+1)/2+i+1); }
    static void mulAv(int n, double[] v, double[] r) { for(int i=0;i<n;i++){r[i]=0;for(int j=0;j<n;j++)r[i]+=A(i,j)*v[j];} }
    static void mulAtv(int n, double[] v, double[] r) { for(int i=0;i<n;i++){r[i]=0;for(int j=0;j<n;j++)r[i]+=A(j,i)*v[j];} }
    static void mulAtAv(int n, double[] v, double[] r, double[] t) { mulAv(n,v,t); mulAtv(n,t,r); }
    public static void main(String[] args) {
        int n = Integer.parseInt(args[0]);
        double[] u = new double[n], v = new double[n], t = new double[n];
        for (int i=0;i<n;i++) u[i]=1;
        for (int i=0;i<10;i++){mulAtAv(n,u,v,t);mulAtAv(n,v,u,t);}
        double vBv=0,vv=0;
        for(int i=0;i<n;i++){vBv+=u[i]*v[i];vv+=v[i]*v[i];}
        System.out.printf("%.9f%n",Math.sqrt(vBv/vv));
    }
}
