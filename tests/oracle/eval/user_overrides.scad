module children() echo("user children");
module m() children();
m() sphere();
module intersection_for(x) echo("user ifor", x);
intersection_for(i = [1:2]) cube(i);
