module m(a) children(a);
m([0:20000]) { cube(); sphere(); }
m([0:0:1]) { cube(); sphere(); }
m([1:0:1]) { cube(); sphere(); }
m([0:1/0:1]) { cube(); sphere(); }
m([1:-1:0]) { cube(); sphere(); }
m([0:0.5:1]) { cube(); sphere(); }
m([-1:1]) { cube(); sphere(); }
m(1.7) { cube(); sphere(); }
m(-0.5) { cube(); sphere(); }
m(1/0) { cube(); sphere(); }
m(2) { cube(); sphere(); }
m("a") { cube(); sphere(); }
m(undef) { cube(); sphere(); }
m([0,"a",1]) { cube(); sphere(); }
m([[0]]) { cube(); sphere(); }
m([0:1:4294967298]) { cube(); sphere(); }
module two() children(0, 1); two() { cube(); sphere(); }
module named() children(index = 1); named() { cube(); sphere(); }
module none() children([0:$children-1]); none();
