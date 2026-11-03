function revise(){
document.querySelectorAll('.feature-grid-card').forEach((article,i)=>{if(article.dataset.sequence)return;article.dataset.sequence='yes';const copy=document.createElement('div');copy.className='feature-copy';copy.append(article.querySelector('h3'),article.querySelector('.feature-grid-description'));article.append(copy);});
const card=document.querySelector('.feature-grid-card:last-child');if(card){const title=card.querySelector('h3');if(title&&title.textContent!=='See what your agent changed')title.textContent='See what your agent changed';const desc=card.querySelector('.feature-grid-description');if(desc&&desc.textContent!=='Your agent shows its changes in a captioned walkthrough.')desc.textContent='Your agent shows its changes in a captioned walkthrough.';const frame=card.querySelector('iframe');if(frame&&frame.title!=='Agent walkthrough — illustrative animation')frame.title='Agent walkthrough — illustrative animation'}

for(const [selector,label] of [['.dancing-cutout','Simon dancing'],['.dog-cutout','Olivia cuddling<br>a dog']]){const person=document.querySelector(selector);if(person){person.querySelector('.person-name')?.remove();const caption=person.querySelector('.cutout-ink');if(caption&&caption.innerHTML!==label)caption.innerHTML=label}}

const dialog=document.querySelector('dialog[data-dialog="social"]');
if(dialog&&!dialog.dataset.revision){const olivia=dialog.querySelector('a[href*="olivia"]'),simon=dialog.querySelector('a[href*="simon"]');if(olivia&&simon){dialog.dataset.revision='yes';olivia.textContent='Olivia';simon.textContent='Simon';const row=document.createElement('div');row.className='social-names';olivia.parentElement.before(row);row.append(olivia.parentElement,simon.parentElement)}}
}
function start(){revise();const observer=new MutationObserver(revise);observer.observe(document.body,{childList:true,subtree:true});window.addEventListener('pagehide',()=>observer.disconnect(),{once:true})}
if(document.readyState==='complete')start();else window.addEventListener('load',start,{once:true});
