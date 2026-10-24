import {errors} from '@wisp/context-values';
for(let i=0;i<8;i++){const value=errors.create();if(errors.describe(value)!=='fixture error')throw Error('wrong typed message');}
console.log('Node error contexts: typed message passed');
