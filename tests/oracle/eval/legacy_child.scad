echo(version(1), search("a", "abc", 1, 2, 3), search("a", "abc", 0, 0, 0));
module a() child([1, 0]);
a() { cube(1); sphere(2); }
module b() child("x");
b() { cube(1); sphere(2); }
module d() child([0:1]);
d() { cube(1); sphere(2); }
