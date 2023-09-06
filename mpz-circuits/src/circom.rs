#![allow(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use crate::{
    components::{Feed, GateType, Node},
    types::ValueType,
    Circuit, CircuitBuilder,
};

use regex::{Captures, Regex};
use std::collections::HashMap;

const VERSION: &'static str = env!("CARGO_PKG_VERSION");

use circom_parser;

use ansi_term::Colour;
use circom_compiler::compiler_interface;
use circom_compiler::compiler_interface::{Config, VCP};
use circom_program_structure::error_definition::Report;
use circom_program_structure::error_code::ReportCode;
use circom_program_structure::file_definition::FileLibrary;
// use crate::VERSION;


// use circom_compiler::hir::very_concrete_program::VCP;
use circom_constraint_writers::debug_writer::DebugWriter;
use circom_constraint_writers::ConstraintExporter;
use circom_program_structure::program_archive::ProgramArchive;

use std::path::PathBuf;

// use super::input_user::Input;
// use circom_program_structure::error_definition::Report;
// use circom_program_structure::program_archive::ProgramArchive;
// use crate::VERSION;

// use circom_program_structure::error_definition::Report;
// use circom_program_structure::program_archive::ProgramArchive;
use circom_type_analysis::check_types::check_types;

#[allow(missing_docs)]
pub struct CompilerConfig {
    pub js_folder: String,
    pub wasm_name: String,
    pub wat_file: String,
    pub wasm_file: String,
    pub c_folder: String,
    pub c_run_name: String,
    pub c_file: String,
    pub dat_file: String,
    pub wat_flag: bool,
    pub wasm_flag: bool,
    pub c_flag: bool,
    pub debug_output: bool,
    pub produce_input_log: bool,
    pub vcp: VCP,
}


pub fn compile(config: CompilerConfig) -> Result<(), ()> {


    if config.c_flag || config.wat_flag || config.wasm_flag{
        let circuit = compiler_interface::run_compiler(
            config.vcp,
            Config { debug_output: config.debug_output, produce_input_log: config.produce_input_log, wat_flag: config.wat_flag },
            VERSION
        )?;
    
        if config.c_flag {
            compiler_interface::write_c(&circuit, &config.c_folder, &config.c_run_name, &config.c_file, &config.dat_file)?;
            println!(
                "{} {} and {}",
                Colour::Green.paint("Written successfully:"),
                config.c_file,
                config.dat_file
            );
            println!(
                "{} {}/{}, {}, {}, {}, {}, {}, {} and {}",
                Colour::Green.paint("Written successfully:"),
            &config.c_folder,
                "main.cpp".to_string(),
                "circom.hpp".to_string(),
                "calcwit.hpp".to_string(),
                "calcwit.cpp".to_string(),
                "fr.hpp".to_string(),
                "fr.cpp".to_string(),
                "fr.asm".to_string(),
                "Makefile".to_string()
            );
        }
    
        match (config.wat_flag, config.wasm_flag) {
            (true, true) => {
                compiler_interface::write_wasm(&circuit, &config.js_folder, &config.wasm_name, &config.wat_file)?;
                println!("{} {}", Colour::Green.paint("Written successfully:"), config.wat_file);
                let result = wat_to_wasm(&config.wat_file, &config.wasm_file);
                match result {
                    Result::Err(report) => {
                        Report::print_reports(&[report], &FileLibrary::new());
                        return Err(());
                    }
                    Result::Ok(()) => {
                        println!("{} {}", Colour::Green.paint("Written successfully:"), config.wasm_file);
                    }
                }
            }
            (false, true) => {
                compiler_interface::write_wasm(&circuit,  &config.js_folder, &config.wasm_name, &config.wat_file)?;
                let result = wat_to_wasm(&config.wat_file, &config.wasm_file);
                std::fs::remove_file(&config.wat_file).unwrap();
                match result {
                    Result::Err(report) => {
                        Report::print_reports(&[report], &FileLibrary::new());
                        return Err(());
                    }
                    Result::Ok(()) => {
                        println!("{} {}", Colour::Green.paint("Written successfully:"), config.wasm_file);
                    }
                }
            }
            (true, false) => {
                compiler_interface::write_wasm(&circuit,  &config.js_folder, &config.wasm_name, &config.wat_file)?;
                println!("{} {}", Colour::Green.paint("Written successfully:"), config.wat_file);
            }
            (false, false) => {}
        }
    }
    

    Ok(())
}


fn wat_to_wasm(wat_file: &str, wasm_file: &str) -> Result<(), Report> {
    use std::fs::read_to_string;
    use std::fs::File;
    use std::io::BufWriter;
    use std::io::Write;
    use wast::Wat;
    use wast::parser::{self, ParseBuffer};

    let wat_contents = read_to_string(wat_file).unwrap();
    let buf = ParseBuffer::new(&wat_contents).unwrap();
    let result_wasm_contents = parser::parse::<Wat>(&buf);
    match result_wasm_contents {
        Result::Err(error) => {
            Result::Err(Report::error(
                format!("Error translating the circuit from wat to wasm.\n\nException encountered when parsing WAT: {}", error),
                ReportCode::ErrorWat2Wasm,
            ))
        }
        Result::Ok(mut wat) => {
            let wasm_contents = wat.module.encode();
            match wasm_contents {
                Result::Err(error) => {
                    Result::Err(Report::error(
                        format!("Error translating the circuit from wat to wasm.\n\nException encountered when encoding WASM: {}", error),
                        ReportCode::ErrorWat2Wasm,
                    ))
                }
                Result::Ok(wasm_contents) => {
                    let file = File::create(wasm_file).unwrap();
                    let mut writer = BufWriter::new(file);
                    writer.write_all(&wasm_contents).map_err(|_err| Report::error(
                        format!("Error writing the circuit. Exception generated: {}", _err),
                        ReportCode::ErrorWat2Wasm,
                    ))?;
                    writer.flush().map_err(|_err| Report::error(
                        format!("Error writing the circuit. Exception generated: {}", _err),
                        ReportCode::ErrorWat2Wasm,
                    ))?;
                    Ok(())
                }
            }
        }
    }
}

pub struct ExecutionConfig {
    pub r1cs: String,
    pub sym: String,
    pub json_constraints: String,
    pub no_rounds: usize,
    pub flag_s: bool,
    pub flag_f: bool,
    pub flag_p: bool,
    pub flag_old_heuristics:bool,
    pub flag_verbose: bool,
    pub inspect_constraints_flag: bool,
    pub sym_flag: bool,
    pub r1cs_flag: bool,
    pub json_substitution_flag: bool,
    pub json_constraint_flag: bool,
    pub prime: String,
}

pub fn execute_project(
    program_archive: ProgramArchive,
    config: ExecutionConfig,
) -> Result<VCP, ()> {
    use circom_constraint_generation::{build_circuit, BuildConfig};
    let debug = DebugWriter::new(config.json_constraints).unwrap();
    let build_config = BuildConfig {
        no_rounds: config.no_rounds,
        flag_json_sub: config.json_substitution_flag,
        flag_s: config.flag_s,
        flag_f: config.flag_f,
        flag_p: config.flag_p,
        flag_verbose: config.flag_verbose,
        inspect_constraints: config.inspect_constraints_flag,
        flag_old_heuristics: config.flag_old_heuristics,
        prime : config.prime,
    };
    let custom_gates = program_archive.custom_gates;
    let (exporter, vcp) = build_circuit(program_archive, build_config)?;
    if config.r1cs_flag {
        generate_output_r1cs(&config.r1cs, exporter.as_ref(), custom_gates)?;
    }
    if config.sym_flag {
        generate_output_sym(&config.sym, exporter.as_ref())?;
    }
    if config.json_constraint_flag {
        generate_json_constraints(&debug, exporter.as_ref())?;
    }
    Result::Ok(vcp)
}

fn generate_output_r1cs(file: &str, exporter: &dyn ConstraintExporter, custom_gates: bool) -> Result<(), ()> {
    if let Result::Ok(()) = exporter.r1cs(file, custom_gates) {
        println!("{} {}", Colour::Green.paint("Written successfully:"), file);
        Result::Ok(())
    } else {
        eprintln!("{}", Colour::Red.paint("Could not write the output in the given path"));
        Result::Err(())
    }
}

fn generate_output_sym(file: &str, exporter: &dyn ConstraintExporter) -> Result<(), ()> {
    if let Result::Ok(()) = exporter.sym(file) {
        println!("{} {}", Colour::Green.paint("Written successfully:"), file);
        Result::Ok(())
    } else {
        eprintln!("{}", Colour::Red.paint("Could not write the output in the given path"));
        Result::Err(())
    }
}

fn generate_json_constraints(
    debug: &DebugWriter,
    exporter: &dyn ConstraintExporter,
) -> Result<(), ()> {
    if let Ok(()) = exporter.json_constraints(&debug) {
        println!("{} {}", Colour::Green.paint("Constraints written in:"), debug.json_constraints);
        Result::Ok(())
    } else {
        eprintln!("{}", Colour::Red.paint("Could not write the output in the given path"));
        Result::Err(())
    }
}

// use std::path::PathBuf;

pub struct Input {
    pub input_program: PathBuf,
    pub out_r1cs: PathBuf,
    pub out_json_constraints: PathBuf,
    pub out_wat_code: PathBuf,
    pub out_wasm_code: PathBuf,
    pub out_wasm_name: String,
    pub out_js_folder: PathBuf,
    pub out_c_run_name: String,
    pub out_c_folder: PathBuf,
    pub out_c_code: PathBuf,
    pub out_c_dat: PathBuf,
    pub out_sym: PathBuf,
    //pub field: &'static str,
    pub c_flag: bool,
    pub wasm_flag: bool,
    pub wat_flag: bool,
    pub r1cs_flag: bool,
    pub sym_flag: bool,
    pub json_constraint_flag: bool,
    pub json_substitution_flag: bool,
    pub main_inputs_flag: bool,
    pub print_ir_flag: bool,
    pub fast_flag: bool,
    pub reduced_simplification_flag: bool,
    pub parallel_simplification_flag: bool,
    pub flag_old_heuristics: bool,
    pub inspect_constraints_flag: bool,
    pub no_rounds: usize,
    pub flag_verbose: bool,
    pub prime: String,
    pub link_libraries : Vec<PathBuf>
}


const R1CS: &'static str = "r1cs";
const WAT: &'static str = "wat";
const WASM: &'static str = "wasm";
const CPP: &'static str = "cpp";
const JS: &'static str = "js";
const DAT: &'static str = "dat";
const SYM: &'static str = "sym";
const JSON: &'static str = "json";


impl Input {
    pub fn new() -> Result<Input, ()> {
        // use SimplificationStyle;
        let matches = view();
        let input = get_input(&matches)?;
        let file_name = input.file_stem().unwrap().to_str().unwrap().to_string();
        let output_path = get_output_path(&matches)?;
        let output_c_path = Input::build_folder(&output_path, &file_name, CPP);
        let output_js_path = Input::build_folder(&output_path, &file_name, JS);
        let o_style = get_simplification_style(&matches)?;
        let link_libraries = get_link_libraries(&matches);
        Result::Ok(Input {
            //field: P_BN128,
            input_program: input,
            out_r1cs: Input::build_output(&output_path, &file_name, R1CS),
            out_wat_code: Input::build_output(&output_js_path, &file_name, WAT),
            out_wasm_code: Input::build_output(&output_js_path, &file_name, WASM),
	        out_js_folder: output_js_path.clone(),
	        out_wasm_name: file_name.clone(),
	        out_c_folder: output_c_path.clone(),
	        out_c_run_name: file_name.clone(),
            out_c_code: Input::build_output(&output_c_path, &file_name, CPP),
            out_c_dat: Input::build_output(&output_c_path, &file_name, DAT),
            out_sym: Input::build_output(&output_path, &file_name, SYM),
            out_json_constraints: Input::build_output(
                &output_path,
                &format!("{}_constraints", file_name),
                JSON,
            ),
            wat_flag:get_wat(&matches),
            wasm_flag: get_wasm(&matches),
            c_flag: get_c(&matches),
            r1cs_flag: get_r1cs(&matches),
            sym_flag: get_sym(&matches),
            main_inputs_flag: get_main_inputs_log(&matches),
            json_constraint_flag: get_json_constraints(&matches),
            json_substitution_flag: get_json_substitutions(&matches),
            print_ir_flag: get_ir(&matches),
            no_rounds: if let SimplificationStyle::O2(r) = o_style { r } else { 0 },
            fast_flag: o_style == SimplificationStyle::O0,
            reduced_simplification_flag: o_style == SimplificationStyle::O1,
            parallel_simplification_flag: get_parallel_simplification(&matches),
            inspect_constraints_flag: get_inspect_constraints(&matches),
            flag_old_heuristics: get_flag_old_heuristics(&matches),
            flag_verbose: get_flag_verbose(&matches), 
            prime: get_prime(&matches)?,
            link_libraries
        })
    }

    fn build_folder(output_path: &PathBuf, filename: &str, ext: &str) -> PathBuf {
        let mut file = output_path.clone();
	    let folder_name = format!("{}_{}",filename,ext);
	    file.push(folder_name);
	    file
    }
    
    fn build_output(output_path: &PathBuf, filename: &str, ext: &str) -> PathBuf {
        let mut file = output_path.clone();
        file.push(format!("{}.{}",filename,ext));
        file
    }

    pub fn get_link_libraries(&self) -> &Vec<PathBuf> {
        &self.link_libraries
    }

    pub fn input_file(&self) -> &str {
        &self.input_program.to_str().unwrap()
    }
    pub fn r1cs_file(&self) -> &str {
        self.out_r1cs.to_str().unwrap()
    }
    pub fn sym_file(&self) -> &str {
        self.out_sym.to_str().unwrap()
    }
    pub fn wat_file(&self) -> &str {
        self.out_wat_code.to_str().unwrap()
    }
    pub fn wasm_file(&self) -> &str {
        self.out_wasm_code.to_str().unwrap()
    }
    pub fn js_folder(&self) -> &str {
        self.out_js_folder.to_str().unwrap()
    }
    pub fn wasm_name(&self) -> String {
        self.out_wasm_name.clone()
    }

    pub fn c_folder(&self) -> &str {
        self.out_c_folder.to_str().unwrap()
    }
    pub fn c_run_name(&self) -> String {
        self.out_c_run_name.clone()
    }

    pub fn c_file(&self) -> &str {
        self.out_c_code.to_str().unwrap()
    }
    pub fn dat_file(&self) -> &str {
        self.out_c_dat.to_str().unwrap()
    }
    pub fn json_constraints_file(&self) -> &str {
        self.out_json_constraints.to_str().unwrap()
    }
    pub fn wasm_flag(&self) -> bool {
        self.wasm_flag
    }
    pub fn wat_flag(&self) -> bool {
        self.wat_flag
    }
    pub fn c_flag(&self) -> bool {
        self.c_flag
    }
    pub fn unsimplified_flag(&self) -> bool {
        self.fast_flag
    }
    pub fn r1cs_flag(&self) -> bool {
        self.r1cs_flag
    }
    pub fn json_constraints_flag(&self) -> bool {
        self.json_constraint_flag
    }
    pub fn json_substitutions_flag(&self) -> bool {
        self.json_substitution_flag
    }
    pub fn main_inputs_flag(&self) -> bool {
        self.main_inputs_flag
    }
    pub fn sym_flag(&self) -> bool {
        self.sym_flag
    }
    pub fn print_ir_flag(&self) -> bool {
        self.print_ir_flag
    }
    pub fn inspect_constraints_flag(&self) -> bool {
        self.inspect_constraints_flag
    }
    pub fn flag_verbose(&self) -> bool {
        self.flag_verbose
    }
    pub fn reduced_simplification_flag(&self) -> bool {
        self.reduced_simplification_flag
    }
    pub fn parallel_simplification_flag(&self) -> bool {
        self.parallel_simplification_flag
    }
    pub fn flag_old_heuristics(&self) -> bool {
        self.flag_old_heuristics
    }
    pub fn no_rounds(&self) -> usize {
        self.no_rounds
    }
    pub fn prime(&self) -> String{
        self.prime.clone()
    }
}

    // use ansi_term::Colour;
    use clap::{App, Arg, ArgMatches};
    use std::path::{Path};

    // use super::VERSION;
    // use crate::VERSION;

    pub fn get_input(matches: &ArgMatches) -> Result<PathBuf, ()> {
        let route = Path::new(matches.value_of("input").unwrap()).to_path_buf();
        if route.is_file() {
            Result::Ok(route)
        } else {
            let route = if route.to_str().is_some() { ": ".to_owned() + route.to_str().unwrap()} else { "".to_owned() };
            Result::Err(eprintln!("{}", Colour::Red.paint("Input file does not exist".to_owned() + &route)))
        }
    }

    pub fn get_output_path(matches: &ArgMatches) -> Result<PathBuf, ()> {
        let route = Path::new(matches.value_of("output").unwrap()).to_path_buf();
        if route.is_dir() {
            Result::Ok(route)
        } else {
            Result::Err(eprintln!("{}", Colour::Red.paint("invalid output path")))
        }
    }

    #[derive(Copy, Clone, Eq, PartialEq)]
    pub enum SimplificationStyle { O0, O1, O2(usize) }
    pub fn get_simplification_style(matches: &ArgMatches) -> Result<SimplificationStyle, ()> {

        let o_0 = matches.is_present("no_simplification");
        let o_1 = matches.is_present("reduced_simplification");
        let o_2 = matches.is_present("full_simplification");
        let o_2round = matches.is_present("simplification_rounds");
        match (o_0, o_1, o_2round, o_2) {
            (true, _, _, _) => Ok(SimplificationStyle::O0),
            (_, true, _, _) => Ok(SimplificationStyle::O1),
            (_, _, true,  _) => {
                let o_2_argument = matches.value_of("simplification_rounds").unwrap();
                let rounds_r = usize::from_str_radix(o_2_argument, 10);
                if let Result::Ok(no_rounds) = rounds_r { 
                    if no_rounds == 0 { Ok(SimplificationStyle::O1) }
                    else {Ok(SimplificationStyle::O2(no_rounds))}} 
                else { Result::Err(eprintln!("{}", Colour::Red.paint("invalid number of rounds"))) }
            },
            
            (false, false, false, true) => Ok(SimplificationStyle::O2(usize::MAX)),
            (false, false, false, false) => Ok(SimplificationStyle::O2(usize::MAX)),
        }
    }

    pub fn get_json_constraints(matches: &ArgMatches) -> bool {
        matches.is_present("print_json_c")
    }

    pub fn get_json_substitutions(matches: &ArgMatches) -> bool {
        matches.is_present("print_json_sub")
    }

    pub fn get_sym(matches: &ArgMatches) -> bool {
        matches.is_present("print_sym")
    }

    pub fn get_r1cs(matches: &ArgMatches) -> bool {
        matches.is_present("print_r1cs")
    }

    pub fn get_wasm(matches: &ArgMatches) -> bool {
        matches.is_present("print_wasm")
    }

    pub fn get_wat(matches: &ArgMatches) -> bool {
        matches.is_present("print_wat")
    }

    pub fn get_c(matches: &ArgMatches) -> bool {
        matches.is_present("print_c")
    }

    pub fn get_main_inputs_log(matches: &ArgMatches) -> bool {
        matches.is_present("main_inputs_log")
    }

    pub fn get_parallel_simplification(matches: &ArgMatches) -> bool {
        matches.is_present("parallel_simplification")
    }

    pub fn get_ir(matches: &ArgMatches) -> bool {
        matches.is_present("print_ir")
    }
    pub fn get_inspect_constraints(matches: &ArgMatches) -> bool {
        matches.is_present("inspect_constraints")
    }

    pub fn get_flag_verbose(matches: &ArgMatches) -> bool {
        matches.is_present("flag_verbose")
    }

    pub fn get_flag_old_heuristics(matches: &ArgMatches) -> bool {
        matches.is_present("flag_old_heuristics")
    }
    pub fn get_prime(matches: &ArgMatches) -> Result<String, ()> {
        
        match matches.is_present("prime"){
            true => 
               {
                   let prime_value = matches.value_of("prime").unwrap();
                   if prime_value == "bn128"
                      || prime_value == "bls12381"
                      || prime_value == "goldilocks"
                      || prime_value == "grumpkin"
                      || prime_value == "pallas"
                      || prime_value == "vesta"
                      {
                        Ok(String::from(matches.value_of("prime").unwrap()))
                    }
                    else{
                        Result::Err(eprintln!("{}", Colour::Red.paint("invalid prime number")))
                    }
               }
               
            false => Ok(String::from("bn128")),
        }
    }

    pub fn view() -> ArgMatches<'static> {
        App::new("circom compiler")
            .version(VERSION)
            .author("IDEN3")
            .about("Compiler for the circom programming language")
            .arg(
                Arg::with_name("input")
                    .multiple(false)
                    .default_value("./circuit.circom")
                    .help("Path to a circuit with a main component"),
            )
            .arg(
                Arg::with_name("no_simplification")
                    .long("O0")
                    .hidden(false)
                    .takes_value(false)
                    .help("No simplification is applied")
                    .display_order(420)
            )
            .arg(
                Arg::with_name("reduced_simplification")
                    .long("O1")
                    .hidden(false)
                    .takes_value(false)
                    .help("Only applies var to var and var to constant simplification")
                    .display_order(460)
            )
            .arg(
                Arg::with_name("full_simplification")
                    .long("O2")
                    .takes_value(false)
                    .hidden(false)
                    .help("Full constraint simplification")
                    .display_order(480)
            )
            .arg(
                Arg::with_name("simplification_rounds")
                    .long("O2round")
                    .takes_value(true)
                    .hidden(false)
                    .help("Maximum number of rounds of the simplification process")
                    .display_order(500)
            )
            .arg(
                Arg::with_name("output")
                    .short("o")
                    .long("output")
                    .takes_value(true)
                    .default_value(".")
                    .display_order(1)
                    .help("Path to the directory where the output will be written"),
            )
            .arg(
                Arg::with_name("print_json_c")
                    .long("json")
                    .takes_value(false)
                    .display_order(120)
                    .help("Outputs the constraints in json format"),
            )
            .arg(
                Arg::with_name("print_ir")
                    .long("irout")
                    .takes_value(false)
                    .hidden(true)
                    .display_order(360)
                    .help("Outputs the low-level IR of the given circom program"),
            )
            .arg(
                Arg::with_name("inspect_constraints")
                    .long("inspect")
                    .takes_value(false)
                    .display_order(801)
                    .help("Does an additional check over the constraints produced"),
            )
            .arg(
                Arg::with_name("print_json_sub")
                    .long("jsons")
                    .takes_value(false)
                    .hidden(true)
                    .display_order(100)
                    .help("Outputs the substitution in json format"),
            )
            .arg(
                Arg::with_name("print_sym")
                    .long("sym")
                    .takes_value(false)
                    .display_order(60)
                    .help("Outputs witness in sym format"),
            )
            .arg(
                Arg::with_name("print_r1cs")
                    .long("r1cs")
                    .takes_value(false)
                    .display_order(30)
                    .help("Outputs the constraints in r1cs format"),
            )
            .arg(
                Arg::with_name("print_wasm")
                    .long("wasm")
                    .takes_value(false)
                    .display_order(90)
                    .help("Compiles the circuit to wasm"),
            )
            .arg(
                Arg::with_name("print_wat")
                    .long("wat")
                    .takes_value(false)
                    .display_order(120)
                    .help("Compiles the circuit to wat"),
            )
            .arg(
                Arg::with_name("link_libraries")
                .short("l")
                .takes_value(true)
                .multiple(true)
                .number_of_values(1)   
                .display_order(330) 
                .help("Adds directory to library search path"),
            )
            .arg(
                Arg::with_name("print_c")
                    .long("c")
                    .short("c")
                    .takes_value(false)
                    .display_order(150)
                    .help("Compiles the circuit to c"),
            )
            .arg(
                Arg::with_name("parallel_simplification")
                    .long("parallel")
                    .takes_value(false)
                    .hidden(true)
                    .display_order(180)
                    .help("Runs non-linear simplification in parallel"),
            )
            .arg(
                Arg::with_name("main_inputs_log")
                    .long("inputs")
                    .takes_value(false)
                    .hidden(true)
                    .display_order(210)
                    .help("Produces a log_inputs.txt file"),
            )
            .arg(
                Arg::with_name("flag_verbose")
                    .long("verbose")
                    .takes_value(false)
                    .display_order(800)
                    .help("Shows logs during compilation"),
            )
            .arg(
                Arg::with_name("flag_old_heuristics")
                    .long("use_old_simplification_heuristics")
                    .takes_value(false)
                    .display_order(980)
                    .help("Applies the old version of the heuristics when performing linear simplification"),
            )
            .arg (
                Arg::with_name("prime")
                    .short("prime")
                    .long("prime")
                    .takes_value(true)
                    .default_value("bn128")
                    .display_order(300)
                    .help("To choose the prime number to use to generate the circuit. Receives the name of the curve (bn128, bls12381, goldilocks, grumpkin, pallas, vesta)"),
            )
            .get_matches()
    }

    pub fn get_link_libraries(matches: &ArgMatches) -> Vec<PathBuf> {
        let mut link_libraries = Vec::new();
        let m = matches.values_of("link_libraries");
        if let Some(paths) = m {
            for path in paths.into_iter() {
                link_libraries.push(Path::new(path).to_path_buf());
            }
        }
        link_libraries
    }

pub fn parse_project(input_info: &Input) -> Result<ProgramArchive, ()> {
    let initial_file = input_info.input_file().to_string();
    let result_program_archive = circom_parser::run_parser(initial_file, VERSION, input_info.get_link_libraries().to_vec());
    match result_program_archive {
        Result::Err((file_library, report_collection)) => {
            Report::print_reports(&report_collection, &file_library);
            Result::Err(())
        }
        Result::Ok((program_archive, warnings)) => {
            Report::print_reports(&warnings, &program_archive.file_library);
            Result::Ok(program_archive)
        }
    }
}


pub fn analyse_project(program_archive: &mut ProgramArchive) -> Result<(), ()> {
    let analysis_result = check_types(program_archive);
    match analysis_result {
        Err(errs) => {
            Report::print_reports(&errs, program_archive.get_file_library());
            Err(())
        }
        Ok(warns) => {
            Report::print_reports(&warns, program_archive.get_file_library());
            Ok(())
        }
    }
}

static GATE_PATTERN: &str = r"(?P<input_count>\d+)\s(?P<output_count>\d+)\s(?P<xref>\d+)\s(?:(?P<yref>\d+)\s)?(?P<zref>\d+)\s(?P<gate>INV|AND|XOR)";

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error(transparent)]
    IOError(#[from] std::io::Error),
    #[error(transparent)]
    ParseIntError(#[from] std::num::ParseIntError),
    #[error("uninitialized feed: {0}")]
    UninitializedFeed(usize),
    #[error("unsupported gate type: {0}")]
    UnsupportedGateType(String),
    #[error(transparent)]
    BuilderError(#[from] crate::BuilderError),
}

impl Circuit {
    /// Parses a circuit in Bristol-fashion format from a file.
    ///
    /// See `https://homes.esat.kuleuven.be/~nsmart/MPC/` for more information.
    ///
    /// # Arguments
    ///
    /// * `filename` - The path to the file to parse.
    /// * `inputs` - The types of the inputs to the circuit.
    /// * `outputs` - The types of the outputs to the circuit.
    ///
    /// # Returns
    ///
    /// The parsed circuit.
    pub fn parse_circom(
        filename: &str,
        inputs: &[ValueType],
        outputs: &[ValueType],
    ) -> Result<(), ()> {
        // let file = std::fs::read_to_string(filename);

    // use compilation_user::CompilerConfig;
    // use execution_user::ExecutionConfig;
    let user_input = Input::new()?;
    let mut program_archive = parse_project(&user_input)?;
    analyse_project(&mut program_archive)?;

    let config = ExecutionConfig {
        no_rounds: user_input.no_rounds(),
        flag_p: user_input.parallel_simplification_flag(),
        flag_s: user_input.reduced_simplification_flag(),
        flag_f: user_input.unsimplified_flag(),
        flag_old_heuristics: user_input.flag_old_heuristics(),
        flag_verbose: user_input.flag_verbose(),
        inspect_constraints_flag: user_input.inspect_constraints_flag(),
        r1cs_flag: user_input.r1cs_flag(),
        json_constraint_flag: user_input.json_constraints_flag(),
        json_substitution_flag: user_input.json_substitutions_flag(),
        sym_flag: user_input.sym_flag(),
        sym: user_input.sym_file().to_string(),
        r1cs: user_input.r1cs_file().to_string(),
        json_constraints: user_input.json_constraints_file().to_string(),
        prime: user_input.prime(),        
    };
    let circuit = execute_project(program_archive, config)?;
    let compilation_config = CompilerConfig {
        vcp: circuit,
        debug_output: user_input.print_ir_flag(),
        c_flag: user_input.c_flag(),
        wasm_flag: user_input.wasm_flag(),
        wat_flag: user_input.wat_flag(),
	    js_folder: user_input.js_folder().to_string(),
	    wasm_name: user_input.wasm_name().to_string(),
	    c_folder: user_input.c_folder().to_string(),
	    c_run_name: user_input.c_run_name().to_string(),
        c_file: user_input.c_file().to_string(),
        dat_file: user_input.dat_file().to_string(),
        wat_file: user_input.wat_file().to_string(),
        wasm_file: user_input.wasm_file().to_string(),
        produce_input_log: user_input.main_inputs_flag(),
    };
    compile(compilation_config)?;

        // let builder = CircuitBuilder::new();

        // let mut feed_ids: Vec<usize> = Vec::new();
        // let mut feed_map: HashMap<usize, Node<Feed>> = HashMap::default();

        // let mut input_len = 0;
        // for input in inputs {
        //     let input = builder.add_input_by_type(input.clone());
        //     for (node, old_id) in input.iter().zip(input_len..input_len + input.len()) {
        //         feed_map.insert(old_id, *node);
        //     }
        //     input_len += input.len();
        // }

        // let mut state = builder.state().borrow_mut();
        // let pattern = Regex::new(GATE_PATTERN).unwrap();
        // for cap in pattern.captures_iter(&file) {
        //     let UncheckedGate {
        //         xref,
        //         yref,
        //         zref,
        //         gate_type,
        //     } = UncheckedGate::parse(cap)?;
        //     feed_ids.push(zref);

        //     match gate_type {
        //         GateType::Xor => {
        //             let new_x = feed_map
        //                 .get(&xref)
        //                 .ok_or(ParseError::UninitializedFeed(xref))?;
        //             let new_y = feed_map
        //                 .get(&yref.unwrap())
        //                 .ok_or(ParseError::UninitializedFeed(yref.unwrap()))?;
        //             let new_z = state.add_xor_gate(*new_x, *new_y);
        //             feed_map.insert(zref, new_z);
        //         }
        //         GateType::And => {
        //             let new_x = feed_map
        //                 .get(&xref)
        //                 .ok_or(ParseError::UninitializedFeed(xref))?;
        //             let new_y = feed_map
        //                 .get(&yref.unwrap())
        //                 .ok_or(ParseError::UninitializedFeed(yref.unwrap()))?;
        //             let new_z = state.add_and_gate(*new_x, *new_y);
        //             feed_map.insert(zref, new_z);
        //         }
        //         GateType::Inv => {
        //             let new_x = feed_map
        //                 .get(&xref)
        //                 .ok_or(ParseError::UninitializedFeed(xref))?;
        //             let new_z = state.add_inv_gate(*new_x);
        //             feed_map.insert(zref, new_z);
        //         }
        //     }
        // }
        // drop(state);
        // feed_ids.sort();

        // for output in outputs.iter().rev() {
        //     let feeds = feed_ids
        //         .drain(feed_ids.len() - output.len()..)
        //         .map(|id| {
        //             *feed_map
        //                 .get(&id)
        //                 .expect("Old feed should be mapped to new feed")
        //         })
        //         .collect::<Vec<Node<Feed>>>();

        //     let output = output.to_bin_repr(&feeds).unwrap();
        //     builder.add_output(output);
        // }

        // Ok(builder.build()?)
        Ok(())
    }
}

struct UncheckedGate {
    xref: usize,
    yref: Option<usize>,
    zref: usize,
    gate_type: GateType,
}

impl UncheckedGate {
    fn parse(captures: Captures) -> Result<Self, ParseError> {
        let xref: usize = captures.name("xref").unwrap().as_str().parse()?;
        let yref: Option<usize> = captures
            .name("yref")
            .map(|yref| yref.as_str().parse())
            .transpose()?;
        let zref: usize = captures.name("zref").unwrap().as_str().parse()?;
        let gate_type = captures.name("gate").unwrap().as_str();

        let gate_type = match gate_type {
            "XOR" => GateType::Xor,
            "AND" => GateType::And,
            "INV" => GateType::Inv,
            _ => return Err(ParseError::UnsupportedGateType(gate_type.to_string())),
        };

        Ok(Self {
            xref,
            yref,
            zref,
            gate_type,
        })
    }
}

#[cfg(test)]
mod tests {
    use mpz_circuits_macros::evaluate;

    use super::*;

    #[test]
    fn test_parse_adder_64() {
        let circ = Circuit::parse(
            "circuits/bristol/adder64_reverse.txt",
            &[ValueType::U64, ValueType::U64],
            &[ValueType::U64],
        )
        .unwrap();

        let output: u64 = evaluate!(circ, fn(1u64, 2u64) -> u64).unwrap();

        assert_eq!(output, 3);
    }

    #[test]
    #[cfg(feature = "aes")]
    #[ignore = "expensive"]
    fn test_parse_aes() {
        use aes::{
            cipher::{BlockEncrypt, KeyInit},
            Aes128,
        };

        let circ = Circuit::parse(
            "circuits/bristol/aes_128_reverse.txt",
            &[
                ValueType::Array(Box::new(ValueType::U8), 16),
                ValueType::Array(Box::new(ValueType::U8), 16),
            ],
            &[ValueType::Array(Box::new(ValueType::U8), 16)],
        )
        .unwrap()
        .reverse_input(0)
        .reverse_input(1)
        .reverse_output(0);

        let key = [0u8; 16];
        let msg = [69u8; 16];

        let ciphertext = evaluate!(circ, fn(key, msg) -> [u8; 16]).unwrap();

        let aes = Aes128::new_from_slice(&key).unwrap();
        let mut expected = msg.into();
        aes.encrypt_block(&mut expected);
        let expected: [u8; 16] = expected.into();

        assert_eq!(ciphertext, expected);
    }

    #[test]
    #[cfg(feature = "sha2")]
    #[ignore = "expensive"]
    fn test_parse_sha() {
        use sha2::compress256;

        let circ = Circuit::parse(
            "circuits/bristol/sha256_reverse.txt",
            &[
                ValueType::Array(Box::new(ValueType::U8), 64),
                ValueType::Array(Box::new(ValueType::U32), 8),
            ],
            &[ValueType::Array(Box::new(ValueType::U32), 8)],
        )
        .unwrap()
        .reverse_inputs()
        .reverse_input(0)
        .reverse_input(1)
        .reverse_output(0);

        static SHA2_INITIAL_STATE: [u32; 8] = [
            0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
            0x5be0cd19,
        ];

        let msg = [69u8; 64];

        let output = evaluate!(circ, fn(SHA2_INITIAL_STATE, msg) -> [u32; 8]).unwrap();

        let mut expected = SHA2_INITIAL_STATE;
        compress256(&mut expected, &[msg.into()]);

        assert_eq!(output, expected);
    }
}
