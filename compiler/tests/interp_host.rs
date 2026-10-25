//! Ендірілген интерпретатор: хост `сыртқы` функциялары, объектілер, қателер.

use std::cell::RefCell;
use std::rc::Rc;

use tulpar::interp::{Host, Interp, Value};

struct TestHost {
    log: Rc<RefCell<Vec<String>>>,
}

impl Host for TestHost {
    fn call(&mut self, name: &str, args: Vec<Value>) -> Result<Value, String> {
        match name {
            "қос_санды" => Ok(Value::Int(args[0].as_int() + args[1].as_int())),
            "хабар" => {
                self.log.borrow_mut().push(args[0].as_str().to_string());
                Ok(Value::Null)
            }
            "қате_бер" => Err(format!("хост қатесі: {}", args[0].as_str())),
            _ => Err(format!("белгісіз: {}", name)),
        }
    }
    fn print(&mut self, text: &str) {
        self.log.borrow_mut().push(text.trim_end().to_string());
    }
}

fn load(src: &str) -> (Interp, Rc<RefCell<Vec<String>>>) {
    let dir = std::env::temp_dir().join(format!("tulpar-interp-test-{}-{:?}", std::process::id(), std::thread::current().id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("m.tlp");
    std::fs::write(&path, src).unwrap();
    let prog = tulpar::loader::load(path.to_str().unwrap()).unwrap_or_else(|d| panic!("{}", d.render()));
    let module = tulpar::check::Checker::check_program(&prog).unwrap_or_else(|d| panic!("{}", d.render()));
    let log = Rc::new(RefCell::new(vec![]));
    let mut it = Interp::new(module, Box::new(TestHost { log: log.clone() }));
    it.run_main().unwrap();
    (it, log)
}

#[test]
fn extern_calls_and_functions() {
    let (mut it, log) = load(
        "сыртқы анықта қос_санды(a: бүтін, b: бүтін) -> бүтін\n\
         сыртқы анықта хабар(т: мәтін)\n\
         анықта есепте(x: бүтін) -> бүтін:\n    хабар(f\"x={x}\")\n    қайтар қос_санды(x, 10) * 2\n",
    );
    let r = it.call("есепте", vec![Value::Int(5)]).unwrap();
    assert_eq!(r.as_int(), 30);
    assert_eq!(log.borrow().as_slice(), ["x=5"]);
}

#[test]
fn host_error_aborts_with_trace() {
    let (mut it, _) = load("сыртқы анықта қате_бер(т: мәтін)\nанықта жұмыс():\n    қате_бер(\"жаман\")\n");
    let e = it.call("жұмыс", vec![]).unwrap_err();
    assert!(e.msg.contains("хост қатесі: жаман"));
    assert!(e.trace.iter().any(|t| t.contains("жұмыс")));
}

#[test]
fn objects_from_host_side() {
    let (mut it, _) = load("класс Жол:\n    аты: мәтін = \"\"\n    саны: нақты = 0.0\nанықта жиын_(ж: Жол) -> нақты:\n    қайтар ж.саны * 2.0\n");
    let cid = it.class_id("Жол").unwrap();
    let o = it.new_object(cid);
    assert!(it.set_field(&o, "саны", Value::Float(2.5)));
    assert!(it.set_field(&o, "аты", Value::str("шай")));
    let r = it.call("жиын_", vec![o.clone()]).unwrap();
    assert_eq!(r.as_f64(), 5.0);
    assert_eq!(it.get_field(&o, "аты").unwrap().as_str(), "шай");
}

#[test]
fn step_limit_stops_infinite_loop() {
    let (mut it, _) = load("анықта цикл():\n    әзірше рас:\n        өткіз\n");
    it.max_steps = 100_000;
    let e = it.call("цикл", vec![]).unwrap_err();
    assert!(e.msg.contains("шегі"));
}

#[test]
fn recursion_limit() {
    let (mut it, _) = load("анықта р(n: бүтін) -> бүтін:\n    қайтар р(n + 1)\n");
    let e = it.call("р", vec![Value::Int(0)]).unwrap_err();
    assert!(e.msg.contains("стек"));
}

#[test]
fn file_access_is_denied() {
    let (mut it, _) = load("анықта оқы_():\n    жаз(файл_оқы(\"/etc/passwd\"))\n");
    let e = it.call("оқы_", vec![]).unwrap_err();
    assert!(e.msg.contains("қолжетімсіз"));
}
