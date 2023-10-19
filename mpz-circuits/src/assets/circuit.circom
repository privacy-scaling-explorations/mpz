pragma circom 2.0.0;

/*This circuit template checks that c is the multiplication of a and b.*/  

template Multiplier2 () {  

   // Declaration of signals 
   signal input garb_a;  
   signal input eval_b;  
   signal i;
   signal output c;  

   // Computation  
   i <== garb_a + eval_b;

   var j = 100;

   var t = i * j;
   c <== garb_a * t;
}

component main = Multiplier2();
