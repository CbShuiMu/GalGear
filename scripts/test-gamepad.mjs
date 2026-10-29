import {test} from 'node:test';
import assert from 'node:assert/strict';
import {direction, nearest} from '../web/gamepad.js';

test('analog deadzone ignores stick drift', () => {
  assert.equal(direction([.2,-.2],[]), '');
});
test('D-pad and analog stick share directional navigation', () => {
  assert.equal(direction([.8,0],[]),'right');
  const buttons=[];buttons[12]={pressed:true};
  assert.equal(direction([0,0],buttons),'up');
});
const rect=(left,top,width=40,height=40)=>({left,top,width,height});
test('horizontal movement stays in a shelf before jumping between rows', () => {
  assert.equal(nearest(rect(0,0),[
    {node:'below',rect:rect(40,100)},
    {node:'right',rect:rect(100,0)},
    {node:'left',rect:rect(-50,0)},
  ],'right'),'right');
});
test('vertical movement can reach controls in the next section', () => {
  assert.equal(nearest(rect(100,0),[
    {node:'below',rect:rect(100,180,180,40)},
    {node:'sidebar',rect:rect(0,100)},
  ],'down'),'below');
});
test('edge movement does not jump to a control behind the current position', () => {
  assert.equal(nearest(rect(0,0),[{node:'behind',rect:rect(-50,0)}],'right'),null);
});
