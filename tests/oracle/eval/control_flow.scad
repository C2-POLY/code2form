for (i = [0:2], j = [0:1]) translate([i, j]) cube(); for (i=[1:0.5:2]) cube(i); for (x = [[1,2],[3,4]]) cube(x[0]); for (c = "ab") echo(c); for (i = 3) cube(i); for (i = undef) cube(); for (i = true) echo(i); for (i = [0:0]) cube(); for (i = [2:1]) echo(i); for (i=[0:-1:-2]) echo(i);
for (i = [0:0.1:0.5]) echo(i); intersection_for (a = [0, 45]) rotate(a) square(2, center = true);
if (1 > 2) cube(); else if (2 > 1) sphere(); else cylinder();
let (a = 2, b = a * 3) cube([a, b, 1]);
nosuch(); cube(); echo(nofn(1)); va = echo("in expr") 5; echo(va); vb = assert(true) 7; echo(vb);
echo(); echo(a=undef); echo(1, a = 2, 3); ve = echo(1, "x") echo(b=2) 3;
!translate([1,0,0]) { cube(); sphere(); }
