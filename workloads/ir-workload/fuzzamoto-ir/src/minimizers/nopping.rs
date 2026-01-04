use super::Minimizer;
use crate::Program;

pub struct NoppingMinimizer {
    program: Program,
    current: Program,
    current_nop: Option<usize>,
}

impl Minimizer for NoppingMinimizer {
    fn new(program: Program) -> Self {
        Self {
            program: program.clone(),
            current: program,
            current_nop: None,
        }
    }
    fn success(&mut self) {
        // Last minimization succeeded, nothing to do here, we'll just move on to the next
        // instruction
    }

    fn failure(&mut self) {
        // Reset the last nopped instruction back to the original
        let current_nop = *self
            .current_nop
            .as_mut()
            .expect("Can't call failure before starting to minimize");
        self.current.instructions[current_nop] = self.program.instructions[current_nop].clone();
    }
}

impl Iterator for NoppingMinimizer {
    type Item = Program;

    fn next(&mut self) -> Option<Self::Item> {
        if self.program.instructions.is_empty() {
            // Nothing to nop or give up
            return None;
        }

        match self.current_nop.as_mut() {
            // Already nopped instruction 0, nothing more todo
            Some(0) => return None,
            // Nop the next instruction
            Some(nop) => *nop -= 1,
            // Nop the first instruction (we go in reverse)
            None => self.current_nop = Some(self.program.instructions.len() - 1),
        }

        let current_nop = *self.current_nop.as_ref().unwrap();

        if !self.current.instructions[current_nop].is_noppable() {
            return self.next();
        }

        self.current.instructions[current_nop].nop();

        Some(self.current.clone())
    }
}
