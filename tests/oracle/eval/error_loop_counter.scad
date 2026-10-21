echo("pre");
echo(len([for (i=0; i<1000005; i=i+1) if (i>999998) let(e=echo("body",i)) i]));
echo("not reached");
cube();
