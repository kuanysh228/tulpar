#include <stdio.h>
double f(double x){return x*x*0.5+1.0/(x+1.0);}
int main(){double s=0;for(long i=0;i<100000000;i++)s+=f(i*0.000001);printf("%.4f\n",s);}
