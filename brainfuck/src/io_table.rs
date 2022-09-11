use super::table::Table;
use crate::util::interpolate_columns;
use crate::util::lift;
use algebra::ExtensionOf;
use algebra::Felt;
use algebra::Multivariate;
use algebra::PrimeFelt;
use algebra::StarkFelt;
use mini_stark::number_theory_transform::inverse_number_theory_transform;
use mini_stark::number_theory_transform::number_theory_transform;

const BASE_WIDTH: usize = 1;
const EXTENSION_WIDTH: usize = 2;

struct IoTable<F, E> {
    num_padded_rows: usize,
    matrix: Vec<[F; BASE_WIDTH]>,
    extended_matrix: Option<Vec<[E; EXTENSION_WIDTH]>>,
}

impl<F: StarkFelt + PrimeFelt, E: Felt + ExtensionOf<F>> IoTable<F, E> {
    // base column
    const VALUE: usize = 0;
    // extension column
    const EVALUATION: usize = 1;

    pub fn new() -> Self {
        IoTable {
            num_padded_rows: 0,
            matrix: Vec::new(),
            extended_matrix: None,
        }
    }

    pub fn len(&self) -> usize {
        self.matrix.len() - self.num_padded_rows
    }

    fn height(&self) -> usize {
        self.matrix.len()
    }

    pub fn pad(&mut self, n: usize) {
        // TODO: seting length here seems kind of strange
        while self.matrix.len() < n {
            self.matrix.push([F::zero()]);
            self.num_padded_rows += 1;
        }
    }

    fn base_boundary_constraints() -> Vec<Multivariate<E>> {
        Vec::new()
    }

    fn extension_boundary_constraints() -> Vec<Multivariate<E>> {
        let variables = Multivariate::variables(2);
        vec![variables[Self::EVALUATION].clone() - variables[Self::VALUE].clone()]
    }

    fn base_transition_constraints() -> Vec<Multivariate<E>> {
        Vec::new()
    }

    fn extension_transition_constraints(challenge: E) -> Vec<Multivariate<E>> {
        let variables = Multivariate::<E>::variables(4);
        let value = variables[Self::VALUE].clone();
        let evaluation = variables[Self::EVALUATION].clone();
        let value_next = variables[Self::VALUE].clone();
        let evaluation_next = variables[Self::EVALUATION].clone();
        vec![evaluation.clone() * challenge + value_next.clone() - evaluation_next.clone()]
    }

    fn extension_terminal_constraints(&self, challenge: E, terminal: E) -> Vec<Multivariate<E>> {
        let variables = Multivariate::<E>::variables(2);
        let offset = challenge.pow(&[self.num_padded_rows as u64]);
        // In every padded row the running evaluation variable is multiplied by another
        // factor `challenge`. We need to multiply `challenge ^ padding_length` to get
        // the value of the evaluation terminal after all `2^k` rows.
        let actual_terminal = terminal * offset;
        vec![variables[Self::EVALUATION].clone() - actual_terminal]
    }

    fn set_matrix(&mut self, matrix: Vec<[F; BASE_WIDTH]>) {
        self.num_padded_rows = 0;
        self.matrix = matrix;
    }

    fn interpolant_degree(&self) -> usize {
        self.matrix.len()
    }

    fn base_lde(&mut self, offset: F, codeword_len: usize) -> Vec<Vec<E>> {
        let polynomials = interpolate_columns(&self.matrix, 0);
        // return the codewords
        polynomials
            .into_iter()
            .map(|poly| {
                let mut coefficients = poly.scale(offset).coefficients;
                coefficients.resize(codeword_len, F::zero());
                lift(number_theory_transform(&coefficients))
            })
            .collect()
    }
}

pub struct OutputTable<F, E>(IoTable<F, E>);

impl<F: StarkFelt + PrimeFelt, E: Felt + ExtensionOf<F>> OutputTable<F, E> {
    pub fn new() -> Self {
        OutputTable(IoTable::new())
    }
}

impl<F: StarkFelt + PrimeFelt, E: Felt + ExtensionOf<F>> Table<F, E> for OutputTable<F, E> {
    const BASE_WIDTH: usize = BASE_WIDTH;
    const EXTENSION_WIDTH: usize = EXTENSION_WIDTH;

    fn len(&self) -> usize {
        self.0.len()
    }

    fn height(&self) -> usize {
        self.0.height()
    }

    fn pad(&mut self, n: usize) {
        self.0.pad(n)
    }

    fn base_boundary_constraints() -> Vec<Multivariate<E>> {
        IoTable::<F, E>::base_boundary_constraints()
    }

    fn base_transition_constraints() -> Vec<Multivariate<E>> {
        IoTable::<F, E>::base_transition_constraints()
    }

    fn extension_boundary_constraints(_challenges: &[E]) -> Vec<Multivariate<E>> {
        IoTable::<F, E>::extension_boundary_constraints()
    }

    fn extension_transition_constraints(challenges: &[E]) -> Vec<Multivariate<E>> {
        let mut challenges_iter = challenges.iter().copied();
        let _a = challenges_iter.next().unwrap();
        let _b = challenges_iter.next().unwrap();
        let _c = challenges_iter.next().unwrap();
        let _d = challenges_iter.next().unwrap();
        let _e = challenges_iter.next().unwrap();
        let _f = challenges_iter.next().unwrap();
        let _alpha = challenges_iter.next().unwrap();
        let _beta = challenges_iter.next().unwrap();
        let _gamma = challenges_iter.next().unwrap();
        let delta = challenges_iter.next().unwrap();
        let _eta = challenges_iter.next().unwrap();
        IoTable::<F, E>::extension_transition_constraints(delta)
    }

    fn extension_terminal_constraints(
        &self,
        challenges: &[E],
        terminals: &[E],
    ) -> Vec<Multivariate<E>> {
        let mut challenges_iter = challenges.iter().copied();
        let _a = challenges_iter.next().unwrap();
        let _b = challenges_iter.next().unwrap();
        let _c = challenges_iter.next().unwrap();
        let _d = challenges_iter.next().unwrap();
        let _e = challenges_iter.next().unwrap();
        let _f = challenges_iter.next().unwrap();
        let _alpha = challenges_iter.next().unwrap();
        let _beta = challenges_iter.next().unwrap();
        let _gamma = challenges_iter.next().unwrap();
        let delta = challenges_iter.next().unwrap();
        let _eta = challenges_iter.next().unwrap();

        let mut terminal_iter = terminals.iter().copied();
        let _processor_instruction_permutation_terminal = terminal_iter.next().unwrap();
        let _processor_memory_permutation_terminal = terminal_iter.next().unwrap();
        let _processor_input_evaluation_terminal = terminal_iter.next().unwrap();
        let processor_output_evaluation_terminal = terminal_iter.next().unwrap();
        let _instruction_evaluation_terminal = terminal_iter.next().unwrap();

        self.0
            .extension_terminal_constraints(delta, processor_output_evaluation_terminal)
    }

    fn interpolant_degree(&self) -> usize {
        self.0.interpolant_degree()
    }

    fn set_matrix(&mut self, matrix: Vec<[F; BASE_WIDTH]>) {
        self.0.set_matrix(matrix)
    }

    fn extend(&mut self, challenges: &[E], initials: &[E]) {
        todo!()
    }

    fn base_lde(&mut self, offset: F, codeword_len: usize) -> Vec<Vec<E>> {
        self.0.base_lde(offset, codeword_len)
    }

    fn extension_lde(&mut self, offset: F, expansion_factor: usize) -> Vec<Vec<E>> {
        todo!()
    }
}

pub struct InputTable<F, E>(IoTable<F, E>);

impl<F: StarkFelt + PrimeFelt, E: Felt + ExtensionOf<F>> InputTable<F, E> {
    pub fn new() -> Self {
        InputTable(IoTable::new())
    }
}

impl<F: StarkFelt + PrimeFelt, E: Felt + ExtensionOf<F>> Table<F, E> for InputTable<F, E> {
    const BASE_WIDTH: usize = BASE_WIDTH;
    const EXTENSION_WIDTH: usize = EXTENSION_WIDTH;

    fn len(&self) -> usize {
        self.0.len()
    }

    fn height(&self) -> usize {
        self.0.height()
    }

    fn pad(&mut self, n: usize) {
        self.0.pad(n)
    }

    fn base_boundary_constraints() -> Vec<Multivariate<E>> {
        IoTable::<F, E>::base_boundary_constraints()
    }

    fn base_transition_constraints() -> Vec<Multivariate<E>> {
        IoTable::<F, E>::base_transition_constraints()
    }

    fn extension_boundary_constraints(_challenges: &[E]) -> Vec<Multivariate<E>> {
        IoTable::<F, E>::extension_boundary_constraints()
    }

    fn extension_transition_constraints(challenges: &[E]) -> Vec<Multivariate<E>> {
        let mut challenges_iter = challenges.iter().copied();
        let _a = challenges_iter.next().unwrap();
        let _b = challenges_iter.next().unwrap();
        let _c = challenges_iter.next().unwrap();
        let _d = challenges_iter.next().unwrap();
        let _e = challenges_iter.next().unwrap();
        let _f = challenges_iter.next().unwrap();
        let _alpha = challenges_iter.next().unwrap();
        let _beta = challenges_iter.next().unwrap();
        let gamma = challenges_iter.next().unwrap();
        let _delta = challenges_iter.next().unwrap();
        let _eta = challenges_iter.next().unwrap();
        IoTable::<F, E>::extension_transition_constraints(gamma)
    }

    fn extension_terminal_constraints(
        &self,
        challenges: &[E],
        terminals: &[E],
    ) -> Vec<Multivariate<E>> {
        let mut challenges_iter = challenges.iter().copied();
        let _a = challenges_iter.next().unwrap();
        let _b = challenges_iter.next().unwrap();
        let _c = challenges_iter.next().unwrap();
        let _d = challenges_iter.next().unwrap();
        let _e = challenges_iter.next().unwrap();
        let _f = challenges_iter.next().unwrap();
        let _alpha = challenges_iter.next().unwrap();
        let _beta = challenges_iter.next().unwrap();
        let gamma = challenges_iter.next().unwrap();
        let _delta = challenges_iter.next().unwrap();
        let _eta = challenges_iter.next().unwrap();

        let mut terminal_iter = terminals.iter().copied();
        let _processor_instruction_permutation_terminal = terminal_iter.next().unwrap();
        let _processor_memory_permutation_terminal = terminal_iter.next().unwrap();
        let processor_input_evaluation_terminal = terminal_iter.next().unwrap();
        let _processor_output_evaluation_terminal = terminal_iter.next().unwrap();
        let _instruction_evaluation_terminal = terminal_iter.next().unwrap();

        self.0
            .extension_terminal_constraints(gamma, processor_input_evaluation_terminal)
    }

    fn interpolant_degree(&self) -> usize {
        self.0.interpolant_degree()
    }

    fn set_matrix(&mut self, matrix: Vec<[F; BASE_WIDTH]>) {
        self.0.set_matrix(matrix)
    }

    fn extend(&mut self, challenges: &[E], initials: &[E]) {
        todo!()
    }

    fn base_lde(&mut self, offset: F, codeword_len: usize) -> Vec<Vec<E>> {
        self.0.base_lde(offset, codeword_len)
    }

    fn extension_lde(&mut self, offset: F, expansion_factor: usize) -> Vec<Vec<E>> {
        todo!()
    }
}
