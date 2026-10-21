a0 = 1;
/*[ G1 ]*/
// desc for nonlit
nl = 1 + 2; // [1:5]
// desc with gap

gap = 2;
/* block desc */
bd = 3;
b2 = 4; // [bad syntax here
b3 = 5; //   [1:2]   trailing
b4 = 6;// [10]
b5 = 7; // [1:2] // second
/* [Group Two] extra */
c1 = 1;
/* not a group */
c2 = 2;
//   spaced desc   
c3 = 3; //
c4 = 4; // "quoted"
c5 = "s"; // [foo, bar]
c6 = 5; // [0:1:2:3]
c7 = 1; // [-5:5]
c8 = [1, 2]; // [0:10]
x = 1; y = 2; // [3:4]
// d1
// d2
z = 9;
c9 = 1; /* [Inline] */ c10 = 2;
d1 = 1; // 10
d2 = 1; // hello [1:2]
d3 = 1; // 1 2
d4 = 1; // abc, def
d5 = 1; // [10:Small, 20:Large]
d6 = 1; // [0.5:0.1:2.5]
d7 = 1; // [a:b]
d8 = 1; // -3
d9 = 1; // [1, "x", y z]
  e1 = 1; // [3:4]
e2 = 1;	// [3:4]
e3 = 1  ;  // [3:4]
e4 =
  1; // [3:4]
e5 = 1; // [ 1 : 2 ]
e6 = true; // [true, false]
e7 = 1; // [1:2]x
e8 = 1; // ["a":"A", "b":"B"]
e9 = 1; // [1e2:2]
f1 = 1; // [abc]
f2 = 1; // hello world
f3 = 1; // 1.5
f4 = 1; // [x:"X"]
f5 = 1; // [.5:1]
f6 = 1; // "a":"b"
f7 = 1; // [[1,2],[3]]
f8 = 1; // []
f9 = 1; //[1:2]
//
g1 = 1;
g2 = undef; g3 = [1, a]; g4 = [1:a]; g5 = -x; g6 = !true; g7 = +1; g8 = --1; g9 = [for(i=[1]) i]; h1 = []; h2 = [[1, [2]]];
s2 = "a;b"; // [x]
w2 = 1; // [x, y
a0 = 5; // [1:9]
