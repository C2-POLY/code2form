sphere($fa = 0/0, $fs = 0/0);
linear_extrude(1, $fa = 0/0) square();
circle($fs = 0/0);
rotate_extrude($fa = 0/0) square();
offset(1, $fs = 0/0) square();
