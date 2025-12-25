#!/usr/bin/env node
/**
 * Migrate all Logicola content sets to LOGOS curriculum JSON format
 */

const fs = require('fs');
const path = require('path');

const SETS_CONFIG = [
    {
        sourceSet: 'setC',
        outDir: '../assets/curriculum/00_logicaffeine/02_propositional',
        prefix: 'C',
        generateExplanation: generatePropositionalExplanation
    },
    {
        sourceSet: 'setJ',
        outDir: '../assets/curriculum/00_logicaffeine/03_modal',
        prefix: 'J',
        generateExplanation: generateModalExplanation
    },
    {
        sourceSet: 'setL',
        outDir: '../assets/curriculum/00_logicaffeine/04_deontic',
        prefix: 'L',
        generateExplanation: generateDeonticExplanation
    },
    {
        sourceSet: 'setN',
        outDir: '../assets/curriculum/00_logicaffeine/05_belief',
        prefix: 'N',
        generateExplanation: generateBeliefExplanation
    },
    {
        sourceSet: 'setQ',
        outDir: '../assets/curriculum/00_logicaffeine/06_informal',
        prefix: 'Q',
        generateExplanation: generateInformalExplanation
    }
];

function parseQuestions(content) {
    const questions = [];
    const lines = content.split('\n');
    let inQuestion = false;
    let currentQuestion = null;
    let inOptions = false;
    let currentOptions = [];

    for (let i = 0; i < lines.length; i++) {
        const line = lines[i];

        const idMatch = line.match(/id:\s*['"]([^'"]+)['"]/);
        if (idMatch && !inQuestion) {
            inQuestion = true;
            currentQuestion = { id: idMatch[1], options: [], explanation: null };
            continue;
        }

        if (inQuestion) {
            const promptMatch = line.match(/prompt:\s*[`'"](.+?)[`'"]\s*,?$/);
            if (promptMatch) {
                currentQuestion.prompt = promptMatch[1].replace(/\\'/g, "'").replace(/\\"/g, '"');
            }

            if (line.includes('options:') && line.includes('[')) {
                inOptions = true;
                currentOptions = [];
            }

            if (inOptions) {
                const labelMatch = line.match(/\{\s*id:\s*(\d+)\s*,\s*label:\s*['"]([^'"]+)['"]/);
                if (labelMatch) {
                    currentOptions[parseInt(labelMatch[1])] = labelMatch[2];
                }

                if (line.includes('],')) {
                    inOptions = false;
                    currentQuestion.options = currentOptions.filter(o => o !== undefined);
                }
            }

            const correctMatch = line.match(/correctId:\s*\[([^\]]+)\]/);
            if (correctMatch) {
                currentQuestion.correctIds = correctMatch[1].split(',').map(s => parseInt(s.trim()));
            }

            if (line.includes('answer:')) {
                const stringMatch = line.match(/answer:\s*['"]([^'"]*)['"]/);
                if (stringMatch) {
                    currentQuestion.explanation = stringMatch[1] || null;
                }
            }

            if (line.trim() === '},') {
                if (currentQuestion.id && currentQuestion.prompt && currentQuestion.options.length > 0 && currentQuestion.correctIds) {
                    questions.push(currentQuestion);
                }
                inQuestion = false;
                currentQuestion = null;
            }
        }
    }

    return questions;
}

function generatePropositionalExplanation(prompt, options, correctIndex) {
    const answer = options[correctIndex];
    if (!answer) return "Select the correct propositional logic translation.";

    let explanation = "In propositional logic: ";

    if (answer.includes('\\cdot')) {
        explanation += "'and' is represented by · (conjunction). ";
    }
    if (answer.includes('\\vee')) {
        explanation += "'or' is represented by ∨ (disjunction). ";
    }
    if (answer.includes('\\supset')) {
        explanation += "'if...then' is represented by ⊃ (conditional). ";
    }
    if (answer.includes('\\sim')) {
        explanation += "'not' is represented by ~ (negation). ";
    }
    if (answer.includes('\\equiv')) {
        explanation += "'if and only if' is represented by ≡ (biconditional). ";
    }

    explanation += "Pay attention to the scope of operators - parentheses matter!";
    return explanation;
}

function generateModalExplanation(prompt, options, correctIndex) {
    const answer = options[correctIndex];
    if (!answer) return "Select the correct modal logic translation.";

    let explanation = "In modal logic: ";

    if (answer.includes('□') || answer.includes('\\Box')) {
        explanation += "□ (box) represents necessity - 'must' or 'necessarily'. ";
    }
    if (answer.includes('◇') || answer.includes('\\Diamond')) {
        explanation += "◇ (diamond) represents possibility - 'can' or 'possibly'. ";
    }

    explanation += "The scope of modal operators affects meaning significantly.";
    return explanation;
}

function generateDeonticExplanation(prompt, options, correctIndex) {
    const answer = options[correctIndex];
    if (!answer) return "Select the correct deontic logic translation.";

    let explanation = "In deontic logic: ";

    if (answer.includes('O') && !answer.includes('\\')) {
        explanation += "O represents obligation - 'ought to' or 'must'. ";
    }
    if (answer.includes('P') && prompt.toLowerCase().includes('permit')) {
        explanation += "P represents permission - 'may' or 'is allowed to'. ";
    }
    if (answer.includes('F')) {
        explanation += "F represents prohibition - 'must not' or 'is forbidden to'. ";
    }

    explanation += "Deontic operators express moral or legal requirements.";
    return explanation;
}

function generateBeliefExplanation(prompt, options, correctIndex) {
    const answer = options[correctIndex];
    if (!answer) return "Select the correct belief logic translation.";

    let explanation = "In belief/epistemic logic: ";

    if (answer.includes('B')) {
        explanation += "B represents belief - 'believes that'. ";
    }
    if (answer.includes('K')) {
        explanation += "K represents knowledge - 'knows that'. ";
    }

    explanation += "The subscript indicates who holds the belief or knowledge.";
    return explanation;
}

function generateInformalExplanation(prompt, options, correctIndex) {
    const answer = options[correctIndex];
    if (!answer) return "Identify what is wrong with this definition.";

    const problems = {
        0: "A definition is TOO BROAD when it includes things that shouldn't be included.",
        1: "A definition is TOO NARROW when it excludes things that should be included.",
        2: "A definition is CIRCULAR when it uses the term being defined (or a close variant) in the definition.",
        3: "A definition uses POORLY UNDERSTOOD TERMS when it explains the unknown using equally unknown concepts.",
        4: "A definition has POOR MATCH IN VAGUENESS when the definiendum and definiens differ in their precision.",
        5: "A definition has POOR MATCH IN EMOTIONAL TONE when it adds evaluative language not present in the original term.",
        6: "A definition HAS NON-ESSENTIAL PROPERTIES when it includes accidental features not central to the concept."
    };

    return problems[correctIndex] || "Evaluate whether this definition captures the essential meaning without being too broad or too narrow.";
}

function migrateSet(config) {
    const sourceFile = path.join(__dirname, `../../external-resources/logicola/content/sets/${config.sourceSet}.ts`);
    const outDir = path.join(__dirname, config.outDir);

    if (!fs.existsSync(sourceFile)) {
        console.log(`Skipping ${config.sourceSet} - file not found`);
        return 0;
    }

    const content = fs.readFileSync(sourceFile, 'utf8');
    const questions = parseQuestions(content);

    if (questions.length === 0) {
        console.log(`Skipping ${config.sourceSet} - no questions parsed`);
        return 0;
    }

    // Clear existing exercise files
    const existingFiles = fs.readdirSync(outDir).filter(f => f.startsWith('ex_'));
    existingFiles.forEach(f => fs.unlinkSync(path.join(outDir, f)));

    let withExplanation = 0;
    let generated = 0;

    questions.forEach((q, i) => {
        const paddedIndex = String(i + 1).padStart(3, '0');
        const filename = `ex_${paddedIndex}.json`;
        const filepath = path.join(outDir, filename);

        let explanation = q.explanation;
        if (!explanation || explanation.trim() === '') {
            const correctIdx = Array.isArray(q.correctIds) ? q.correctIds[0] : q.correctIds;
            explanation = config.generateExplanation(q.prompt, q.options, correctIdx);
            generated++;
        } else {
            withExplanation++;
        }

        const exercise = {
            id: `${config.prefix}_${q.id}`,
            type: "multiple_choice",
            difficulty: 1,
            prompt: q.prompt,
            options: q.options,
            correct: Array.isArray(q.correctIds) ? q.correctIds[0] : q.correctIds,
            hint: null,
            explanation: explanation
        };

        fs.writeFileSync(filepath, JSON.stringify(exercise, null, 2) + '\n');
    });

    console.log(`${config.sourceSet}: Wrote ${questions.length} exercises (${withExplanation} original, ${generated} generated)`);
    return questions.length;
}

// Run all migrations
console.log('Migrating Logicola content sets to LOGOS...\n');

let total = 0;
for (const config of SETS_CONFIG) {
    total += migrateSet(config);
}

console.log(`\nTotal: ${total} exercises migrated`);
