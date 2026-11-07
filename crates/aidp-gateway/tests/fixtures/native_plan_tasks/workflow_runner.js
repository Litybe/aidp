const fs = require("fs");
const path = require("path");

const scriptPath = process.argv[2];
const workspace = process.argv[3];
const holdMilliseconds = Number(process.argv[4]);
const taskPattern = /## Plan task `([^`]+)`/;
const filesHeading = "### Files you own (your write set)";
const bulletPattern = /^- `([^`]+)`$/;

function ownedFiles(prompt) {
  const start = prompt.indexOf(filesHeading);
  const lines = prompt.slice(start + filesHeading.length).split("\n").slice(1);
  const files = [];
  for (const line of lines) {
    const match = bulletPattern.exec(line);
    if (!match) {
      if (files.length > 0) break;
      continue;
    }
    files.push(match[1]);
  }
  return files;
}

async function agent(prompt) {
  const task = taskPattern.exec(prompt);
  if (task) {
    for (const file of ownedFiles(prompt)) {
      const target = path.join(workspace, file);
      fs.mkdirSync(path.dirname(target), { recursive: true });
      fs.writeFileSync(target, task[1] + "\n");
    }
    await new Promise((resolve) => setTimeout(resolve, holdMilliseconds));
  }
  return { status: "completed", summary: "done", outputs: {}, signals: {} };
}

const source = fs.readFileSync(scriptPath, "utf8").replace("export const meta", "const meta");
const run = new Function("agent", "log", "phase", "args", `return (async () => {\n${source}\n})();`);
run(agent, () => {}, () => {}, {}).then((result) => {
  process.stdout.write(JSON.stringify({ status: result.status, completed: result.completed }));
});
