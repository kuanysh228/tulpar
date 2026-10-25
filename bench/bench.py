import time, sys
def collatz(n):
    s = 0
    while n != 1:
        if n % 2 == 0: n //= 2
        else: n = 3 * n + 1
        s += 1
    return s
def fib(n): return n if n < 2 else fib(n-1) + fib(n-2)
def sieve(n):
    p = [True]*(n+1); p[0]=p[1]=False; i=2
    while i*i<=n:
        if p[i]:
            for j in range(i*i,n+1,i): p[j]=False
        i+=1
    return sum(p)
def fl(n):
    s=0.0
    for i in range(n): s += (i*0.000001)**2*0.5 + 1.0/(i*0.000001+1.0)
    return s
which = sys.argv[1]
t=time.time()
if which=="collatz":
    best=0
    for i in range(1,3000000):
        k=collatz(i)
        if k>best: best=k
elif which=="fib": fib(42)
elif which=="sieve": sieve(20000000)
elif which=="float": fl(100000000)
print(which, round(time.time()-t,2))
