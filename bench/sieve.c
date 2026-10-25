#include <stdio.h>
#include <stdlib.h>
int main(){long n=20000000;char*p=malloc(n+1);for(long i=0;i<=n;i++)p[i]=1;p[0]=p[1]=0;for(long i=2;i*i<=n;i++)if(p[i])for(long j=i*i;j<=n;j+=i)p[j]=0;long c=0;for(long i=0;i<=n;i++)if(p[i])c++;printf("%ld\n",c);}
