pub fn int_sqrt(number: u64) -> u64 {
    if number == 0 {
        return 0;
    }
    // First estimate good initial value of the square root as log2(number).
    let mut act_number = number;
    let mut square_root = 1;
    while act_number >= 2 {
        // Double the square root until |square_root * square_root > number|.
        square_root *= 2;
        act_number /= 4;
    }
    // Perform Newton's (or Babylonian) method to find the true floor(sqrt()).
    loop {
        // New |square_root| estimate is computed as the average between
        // |square_root| and |number / square_root|.
        square_root = (square_root + number / square_root) / 2;

        // Note that after the first iteration, the estimate is always going to be
        // larger or equal to the true square root value. Therefore to check
        // convergence, we can simply detect condition when the square of the
        // estimated square root is larger than the input.
        if square_root * square_root <= number {
            break;
        }
    }
    square_root
}
