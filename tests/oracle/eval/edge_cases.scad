text("a\"b\\c\nd"); import("weird \"name\".stl"); surface(file="a\\b.dat");
echo(search([5], [1, 2]), search("ab", ["a", "b", "ab"]), search(["a", 1], ["x", 1, "a"]), search(1, [[1, "a"], [1, "b"]], 0, 0), search("z", [["a", 1]], 1, 0));
echo([function(x) x, [1, function(y, z = 2) y + z]], [undef, true, "q"]);
echo(1 < 2 == true, -2 ^ 2, (-2) ^ 2, 2 ^ 3 ^ 2, 7 % -3, -7 % 3, 5.5 % 2, 1 / 0 * 0, [1, 2, 3] - [1, 1], [] + []);
echo([[1, 2], [3, 4]] * [[5, 6, 7], [8, 9, 10]], [1, 2] * [[1], [2]], [[1, 2, 3]] * [1, 2, 3], [[1], [2]] * [3]);
echo(len([[]]), len(""), len("héllo"), "héllo"[1], str("é", 1e6, 1e-7, 123456.5, 0.5e-5), chr(233), ord("é"), chr([104, 105]));
echo(max(1, 5, 3), min([4, 2, 8]), max([]), min(1), norm([]), norm([3, 4, 12]), cross([1, 2, 3], [4, 5, 6]), lookup(1.5, [[0, 0], [1, 10], [2, 30]]), lookup(-1, [[0, 5], [1, 7]]));
echo(floor(-2.5), round(-0.5), ceil(-0.5), abs(-0), sign(-0.0), sqrt(16), pow(2, -1), exp(0), ln(1), log(1000), atan2(-1, -1), asin(0.5), acos(-1), tan(-45), sin(-30), cos(-60));
echo([0:3][4], [0:3][3], [0:0.25:1], [1:-1:1], [3:1], [0:1:-1], len([for (i = [0:0.1:1]) i]));
echo(is_num(undef), is_bool(0), is_list([0:1]), is_string(""), is_undef(undef[0]), is_function(function() 1));
v = [10, 20, 30]; echo(v.x, v.y, v.z, v.w, "abc".x, [0:2].x);
echo(concat([1], [[2]], undef, "s", [0:2]), [each [0:2]], [for (a = [1, 2]) each [a, a * 10]]);
echo([for (i = [0:2]) if (i % 2 == 0) i else -i], [if (false) 1], [for (x = []) x], [for (i = [0:1], j = [i:1]) [i, j]]);
module deep(n) if (n > 0) deep(n - 1); else echo("bottom", $parent_modules); deep(4);
module modvar(a = 1) { b = a * 2; children(); echo(b); } modvar(3) { echo("child sees", $children); cube(); }
module special() { echo($fn, $fa, $fs, $t, $preview); } special(); special($fn = 5); let($fa = 3) special();
$fs = 0.5; sphere(1); translate([1, 0, 0]) sphere(1, $fn = 6);
cylinder(h = 2, r = 1, center = true, $fn = 20); cylinder(d1 = 4, d2 = 0, h = 3); cylinder(5, 2, 0, true);
polyhedron(points = [[0,0,0],[1,0,0],[0,1,0]], faces = [[0,1,2]]); polygon(points = [[0, 0], [1, 0], [1, 1]], paths = [[0, 1, 2]]);
linear_extrude(height = 5, twist = 30, slices = 10, scale = 0.5, center = true, $fn = 16) circle(1);
rotate_extrude(angle = 270, $fn = 12) translate([2, 0]) circle(0.5); offset(delta = -0.2, chamfer = true) square(2);
minkowski() { cube(); sphere(0.1); } hull() { circle(); translate([2, 0]) circle(); } projection(cut = true) sphere(3); render() difference() { cube(2); sphere(1.2); }
color("SteelBlue", 0.5) cube(); color([0.1, 0.2, 0.3]) sphere(); color("#123") cube(); color("#12345678") cube();
resize([2, 0, 0], auto = [true, true, false]) sphere(); mirror([0, 1, 0]) mirror([1, 0, 1]) cube(); multmatrix([[2, 0, 0, 1], [0, 2, 0, 2], [0, 0, 2, 3], [0, 0, 0, 1]]) cube();
for (i = [0:3]) rotate(i * 90) translate([i, 0]) square(0.5); intersection_for (i = [1:3]) rotate([0, 0, i * 30]) cube(2, center = true);
t = [[0, "a", 5], [1, "b", 6], [2, "ab", 7]];
echo(search("ab", t, 1, 1), search("ab", t, 0, 1), search("b", t, 1, 1), search("ab", t), search("ab", ["a", ["b"], "ab"]), search("ab", [["a"], ["b"]]), search("a", [["a"], ["b", "a"]], 0, 1), search("a", [["a"], ["b", "a"]], 0, 0), search("a", [["a"], ["b", "a"]]), search("x", t, 1, 1), search("x", t, 0, 1), search("x", "abc", 0), search("aa", "banana", 2));
echo([0:3][2], [0:3][3], [0:2:6][3], [0:2:6][2], [0:0][0], [1:1:1][0], [0:0.5:1][2], [5:-1:0][5]);
