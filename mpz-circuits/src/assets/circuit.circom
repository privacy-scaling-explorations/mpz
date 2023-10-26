pragma circom 2.0.0;

/*This circuit template checks that c is the multiplication of a and b.*/  

template Multiplier2 () {  

   // Declaration of signals 
   signal input garb_a;  
   signal input eval_b;  
   var i;
   signal output c;  

   // Computation  
   // i <== garb_a + eval_b;

   // var j = 100;

   // var k = 0;

   // if (garb_a != 3) {
   //    k = k + 1;
   // } else {
   //    j = j + 1;
   // }

   // var t = i * j;
   // c <== garb_a * t;

   i = garb_a + eval_b;
   i = garb_a * eval_b;
   i = garb_a + 100;
   i = eval_b * 20;
   c <== i * i;
}

component main = Multiplier2();
