#include <stdio.h>
long collatz(long n){long s=0;while(n!=1){if(n%2==0)n=n/2;else n=3*n+1;s++;}return s;}
int main(){long best=0;for(long i=1;i<3000000;i++){long k=collatz(i);if(k>best)best=k;}printf("%ld\n",best);}
