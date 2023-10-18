pragma circom 2.0.0;

/*This circuit template checks that c is the multiplication of a and b.*/  

template Multiplier2 () {  

   // Declaration of signals.  
   signal input garb_a;  
   signal input eval_b;  
   signal output c;  

   // Constraints.  
   c <== garb_a * eval_b;  
   garb_a <== eval_b * eval_b;

   var i = 0;

   if (i == 0) {
      c <== garb_a * garb_a; 
   } else {
      eval_b <== c * c;
   }

   while(i < 3){
      i = i + 1;
      c <== garb_a * garb_a; 
   }
}

component main = Multiplier2();
