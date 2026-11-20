# Assignment

**expr.assign.1** `x = e` evaluates `e` and stores its value in the local `x`, whose type `e` must
have (E0201). The assignment has type `()`. Only a local can be assigned to (E0206), and only one
declared `mut` (E0301).

**expr.assign.compound.1** `x op= e`, for `op` one of `+ - * / % & | ^ << >>`, is `x = x op e`, with
the same types and traps.
