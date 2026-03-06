const { invoke } = window.__TAURI__.core;

let greetInputEl;
let greetMsgEl;

async function greet() {
  // Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
  const n1 = parseFloat(number_1.value) || 0;
  const n2 = parseFloat(number_2.value) || 0;

  const greetMsg = await invoke("greet", { name: greetInputEl.value });
  const sum = await invoke("add", { num1: n1, num2: n2 });

  greetMsgEl.textContent = `${greetMsg} Also, ${n1} + ${n2} = ${sum}`;
}

window.addEventListener("DOMContentLoaded", () => {
  greetInputEl = document.querySelector("#greet-input");
  number_1 = document.querySelector("#number_1");
  number_2 = document.querySelector("#number_2");
  greetMsgEl = document.querySelector("#greet-msg");
  document.querySelector("#greet-form").addEventListener("submit", (e) => {
    e.preventDefault();
    greet();
  });
});
