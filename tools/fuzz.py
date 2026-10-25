#!/usr/bin/env python3
"""Дифференциалды фаззер: кездейсоқ Tulpar бағдарламалары -O0, -O2 және Python эталонымен салыстырылады."""
import random, subprocess, sys, os, tempfile, signal

M = (1 << 64)
def w(x):
    x &= M - 1
    return x - M if x >= (1 << 63) else x

class Gen:
    def __init__(self, rnd):
        self.r = rnd
        self.funcs = []  # (name, nparams)
        self.tl, self.py = [], []

    def var(self, vs): return self.r.choice(vs)

    def expr(self, vs, depth=0):
        r = self.r
        if depth > 3 or r.random() < 0.25:
            if r.random() < 0.6: v = self.var(vs); return v, v
            c = r.randint(-20, 100); return (f"({c})" if c < 0 else str(c)), (f"({c})" if c < 0 else str(c))
        k = r.choice(['+', '-', '*', '&', '|', '^', 'div', 'mod', 'shl', 'shr', 'neg', 'tern', 'call', 'min', 'max', 'abs'])
        if k == 'call' and self.funcs:
            name, n = r.choice(self.funcs)
            args = [self.expr(vs, depth + 1) for _ in range(n)]
            return f"{name}({', '.join(a[0] for a in args)})", f"{name}({', '.join(a[1] for a in args)})"
        if k in '+-*&|^' and len(k) == 1:
            a, b = self.expr(vs, depth + 1), self.expr(vs, depth + 1)
            pyop = {'+': 'w(%s + %s)', '-': 'w(%s - %s)', '*': 'w(%s * %s)', '&': '(%s & %s)', '|': '(%s | %s)', '^': '(%s ^ %s)'}[k]
            return f"({a[0]} {k} {b[0]})", pyop % (a[1], b[1])
        if k in ('div', 'mod'):
            a, b = self.expr(vs, depth + 1), self.expr(vs, depth + 1)
            op = '//' if k == 'div' else '%'
            return f"({a[0]} {op} (({b[0]} & 7) + 1))", f"({a[1]} {op} (({b[1]} & 7) + 1))"
        if k == 'shl':
            a, b = self.expr(vs, depth + 1), self.expr(vs, depth + 1)
            return f"({a[0]} << ({b[0]} & 15))", f"w({a[1]} << ({b[1]} & 15))"
        if k == 'shr':
            a, b = self.expr(vs, depth + 1), self.expr(vs, depth + 1)
            return f"({a[0]} >> ({b[0]} & 15))", f"({a[1]} >> ({b[1]} & 15))"
        if k == 'neg':
            a = self.expr(vs, depth + 1); return f"(-{a[0]})", f"w(-{a[1]})"
        if k == 'tern':
            c = self.cond(vs, depth + 1); a, b = self.expr(vs, depth + 1), self.expr(vs, depth + 1)
            return f"({a[0]} егер {c[0]} әйтпесе {b[0]})", f"({a[1]} if {c[1]} else {b[1]})"
        if k in ('min', 'max'):
            a, b = self.expr(vs, depth + 1), self.expr(vs, depth + 1)
            tn = 'мин' if k == 'min' else 'макс'
            return f"{tn}({a[0]}, {b[0]})", f"{k}({a[1]}, {b[1]})"
        if k == 'abs':
            a = self.expr(vs, depth + 1); return f"абс({a[0]})", f"w(abs({a[1]}))"
        return self.expr(vs, depth + 1)

    def cond(self, vs, depth=0):
        r = self.r
        if depth < 3 and r.random() < 0.3:
            a, b = self.cond(vs, depth + 1), self.cond(vs, depth + 1)
            if r.random() < 0.5: return f"({a[0]} және {b[0]})", f"({a[1]} and {b[1]})"
            return f"({a[0]} немесе {b[0]})", f"({a[1]} or {b[1]})"
        if depth < 3 and r.random() < 0.1:
            a = self.cond(vs, depth + 1); return f"(емес {a[0]})", f"(not {a[1]})"
        a, b = self.expr(vs, depth + 1), self.expr(vs, depth + 1)
        op = r.choice(['<', '<=', '>', '>=', '==', '!='])
        return f"({a[0]} {op} {b[0]})", f"({a[1]} {op} {b[1]})"

    def stmts(self, vs, ind, depth, n):
        r = self.r
        for _ in range(n):
            k = r.choice(['asg', 'asg', 'asg', 'if', 'for', 'while', 'aug'])
            pad = '    ' * ind
            if k == 'asg' or depth >= 3:
                v = self.var(vs); e = self.expr(vs)
                self.tl.append(f"{pad}{v} = {e[0]}"); self.py.append(f"{pad}{v} = {e[1]}")
            elif k == 'aug':
                v = self.var(vs); e = self.expr(vs); op = r.choice(['+', '-', '*'])
                self.tl.append(f"{pad}{v} {op}= {e[0]}"); self.py.append(f"{pad}{v} = w({v} {op} {e[1]})")
            elif k == 'if':
                c = self.cond(vs)
                self.tl.append(f"{pad}егер {c[0]}:"); self.py.append(f"{pad}if {c[1]}:")
                self.stmts(vs, ind + 1, depth + 1, r.randint(1, 3))
                if r.random() < 0.6:
                    self.tl.append(f"{pad}әйтпесе:"); self.py.append(f"{pad}else:")
                    self.stmts(vs, ind + 1, depth + 1, r.randint(1, 3))
            elif k == 'for':
                i = f"i{depth}{r.randint(0, 9)}"; n = r.randint(0, 6)
                self.tl.append(f"{pad}әр {i} ішінде аралық({n}):"); self.py.append(f"{pad}for {i} in range({n}):")
                self.stmts(vs + [i], ind + 1, depth + 1, r.randint(1, 3))
            else:
                c = f"k{depth}{r.randint(0, 9)}"; n = r.randint(0, 5)
                self.tl.append(f"{pad}{c} = 0"); self.py.append(f"{pad}{c} = 0")
                self.tl.append(f"{pad}әзірше {c} < {n}:"); self.py.append(f"{pad}while {c} < {n}:")
                self.stmts(vs + [c], ind + 1, depth + 1, r.randint(1, 2))
                self.tl.append(f"{pad}    {c} += 1"); self.py.append(f"{pad}    {c} += 1")

    def program(self):
        r = self.r
        for fi in range(r.randint(1, 4)):
            n = r.randint(1, 4)
            ps = [f"p{j}" for j in range(n)]
            name = f"ф{fi}"
            self.tl.append(f"анықта {name}({', '.join(p + ': бүтін' for p in ps)}) -> бүтін:"); self.py.append(f"def {name}({', '.join(ps)}):")
            locs = ['a', 'b']
            for l in locs:
                self.tl.append(f"    {l} = {r.randint(-5, 5)}"); self.py.append(f"    {l} = {self.tl[-1].split('=')[1].strip()}")
            self.stmts(ps + locs, 1, 1, r.randint(2, 6))
            e = self.expr(ps + locs)
            self.tl.append(f"    қайтар {e[0]}"); self.py.append(f"    return {e[1]}")
            self.funcs.append((name, n))
        gl = ['x', 'y', 'z', 'u']
        for g in gl:
            c = r.randint(-9, 9)
            self.tl.append(f"{g} = {c}"); self.py.append(f"{g} = {c}")
        self.stmts(gl, 0, 0, r.randint(4, 12))
        for g in gl:
            self.tl.append(f"жаз({g})"); self.py.append(f"print({g})")

def main():
    seeds = range(int(sys.argv[1]), int(sys.argv[2]))
    tul = os.environ.get('TULPAR', './target/release/tulpar')
    bad = 0
    with tempfile.TemporaryDirectory() as d:
        for seed in seeds:
            g = Gen(random.Random(seed)); g.program()
            tl, py = '\n'.join(g.tl) + '\n', '\n'.join(g.py) + '\n'
            open(f'{d}/p.tlp', 'w', encoding='utf-8').write(tl)
            ns = {'w': w}
            def on_alarm(*a): raise TimeoutError()
            signal.signal(signal.SIGALRM, on_alarm); signal.setitimer(signal.ITIMER_REAL, 1.0)
            import io, contextlib
            buf = io.StringIO()
            try:
                with contextlib.redirect_stdout(buf):
                    exec(compile(py, 'p', 'exec'), ns)
            except BaseException as e:
                signal.setitimer(signal.ITIMER_REAL, 0)
                continue  # эталон тым ұзақ/құлады — өткіземіз
            signal.setitimer(signal.ITIMER_REAL, 0)
            exp = buf.getvalue()
            for flag in ([], ['-O0']):
                r = subprocess.run([tul, 'run', f'{d}/p.tlp'] + flag, capture_output=True, text=True, timeout=10)
                if r.stdout != exp:
                    bad += 1
                    print(f"=== СЕРІ {seed} {flag} сәйкес емес"); open(f'/tmp/fuzz_fail_{seed}.tlp', 'w', encoding='utf-8').write(tl)
                    print(r.stderr[:300]); break
    print(f"фаззер: {len(seeds)} бағдарлама, сәйкессіздік: {bad}")
    sys.exit(1 if bad else 0)
main()
