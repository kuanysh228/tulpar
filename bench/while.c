#include <stdio.h>
int main(){ long i=0,s=0; while(i<1000000000){ s+=i&7; i++; } printf("%ld\n",s); long n=0,j=0; while(j<300000){ long k=0; while(k<3000){ n+=(j^k)&1; k++; } j++; } printf("%ld\n",n); }
