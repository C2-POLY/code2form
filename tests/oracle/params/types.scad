b = true;
n = 5; // [0:10]
s = "abc";
v = [1, 2, 3];
f = 1.5;
w = "x"; // [x, y, z]
m = [1, "a"];
neg = -2;
u = undef;
echo(b=b, n=n, s=s, v=v, f=f, w=w, m=m, neg=neg, u=u);
echo(is_bool(b), is_num(n), is_string(s), is_list(v), is_num(f), is_string(w), is_list(m), is_num(neg));
